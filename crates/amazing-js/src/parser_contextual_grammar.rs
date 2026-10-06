// Private-brand relational production adapted from boa-dev/boa
// (core/parser/src/parser/expression/mod.rs) @39cd11214ecb366200157dab96b3919e61e9a391, MIT.
use super::*;

impl Parser {
    /// Unlike async-method lookahead, ClassElementName/PropertyName never
    /// starts with `*`. A newline before that token can separate a get/set
    /// field from a following generator method through ASI.
    pub(super) fn nth_starts_accessor_name(&self, index: usize) -> bool {
        self.nth_starts_member_key(index)
            && !matches!(self.nth(index).kind, TokenKind::Punct(Punct::Star))
    }

    /// A field initializer is a new strict initializer-code grammar context.
    /// Both its computed key and initializer preserve enclosing Yield/Await parameters.
    pub(super) fn parse_class_field_initializer(&mut self) -> Result<Expr, SyntaxError> {
        let saved = (self.in_async, self.in_generator, self.in_static_block,
            self.in_class_field_initializer, self.in_formal_params, self.in_new_target,
            self.super_call_allowed, self.super_property_allowed, self.no_in, self.in_function);
        // ClassStaticBlock uses +Await, even though direct await is an early error.
        // Its nested field initializer therefore inherits that grammar parameter.
        self.in_async = saved.0.max(usize::from(saved.2 > 0));
        self.in_static_block = 0;
        self.in_class_field_initializer += 1;
        self.in_formal_params = 0;
        self.in_new_target = 1;
        self.super_call_allowed = false;
        self.super_property_allowed = true;
        self.no_in = false;
        self.in_function += 1;
        let result = self.parse_assignment_expr();
        (self.in_async, self.in_generator, self.in_static_block,
            self.in_class_field_initializer, self.in_formal_params, self.in_new_target,
            self.super_call_allowed, self.super_property_allowed, self.no_in, self.in_function) = saved;
        result
    }

    /// PrivateIdentifier is not a PrimaryExpression. Its sole expression
    /// production is PrivateIdentifier `in` ShiftExpression at relational precedence.
    pub(super) fn parse_private_brand_expression(&mut self, min_prec: u8) -> Result<Expr, SyntaxError> {
        const IN_PRECEDENCE: u8 = 10;
        let token = self.cur().clone();
        if self.no_in || min_prec > IN_PRECEDENCE {
            return Err(self.err("private brand check is not allowed at this precedence"));
        }
        let TokenKind::PrivateIdent(name) = token.kind else { unreachable!() };
        self.bump();
        let name = Ident::new(name, token.span);
        self.use_private(name.clone())?;
        if !self.eat_kw(crate::tokens::Keyword::In) {
            return Err(self.err("private identifier is only valid with `in` (unescaped operator required)"));
        }
        let right = self.parse_binary(IN_PRECEDENCE + 1)?;
        if matches!(right.kind, ExprKind::Arrow(_)) {
            return Err(self.err("private brand in requires a shift-expression right operand"));
        }
        let left = Expr::new(ExprKind::PrivateName(name), token.span);
        let span = Span::new(token.span.start, right.span.end);
        Ok(Expr::new(ExprKind::Binary {op: BinOp::In, left: Box::new(left), right: Box::new(right)}, span))
    }
}
