//! Expression grammar and literals.
use super::*;

impl Parser {
    // ----- expressions: entry points ----------------------------------------

    /// `Expression` (comma sequences allowed) with a configurable no-in
    /// restriction (spec §14.10 "it is a Syntax Error if … Contains in").
    pub(super) fn parse_expression_no_in(&mut self, no_in: bool) -> Result<Expr, SyntaxError> {
        let saved = self.no_in;
        self.no_in = no_in;
        let r = self.parse_sequence();
        self.no_in = saved;
        r
    }

    /// Full expression, sequences allowed, `in` allowed.
    pub(super) fn parse_expression(&mut self) -> Result<Expr, SyntaxError> {
        self.parse_expression_no_in(false)
    }

    pub(super) fn parse_sequence(&mut self) -> Result<Expr, SyntaxError> {
        let start = self.cur_loc();
        let first = self.parse_assignment_expr()?;
        if !self.at_punct(Punct::Comma) {
            return Ok(first);
        }
        let mut items = vec![first];
        while self.eat_punct(Punct::Comma) {
            // A trailing comma (`(a,)`) makes the next assignment parse fail
            // with a positioned "unexpected …" diagnostic.
            items.push(self.parse_assignment_expr()?);
        }
        Ok(Expr::new(ExprKind::Sequence(items), self.span_from(start)))
    }

    /// `AssignmentExpression`: conditional, or an assignment whose left side
    /// is validated as assignable (right-associative).
    pub(super) fn parse_assignment_expr(&mut self) -> Result<Expr, SyntaxError> {
        let start = self.cur_loc();
        let left = self.parse_conditional_expr()?;
        let op = match self.cur().kind {
            TokenKind::Punct(p) => assign_op_of(p),
            _ => None,
        };
        if let Some(op) = op {
            if op == AssignOp::Assign {
                if matches!(left.kind, ExprKind::Assign { .. }) {
                    return Err(self.err_at(left.span.start, "assignment expression is not an assignment target"));
                }
                if !self.strict_script && matches!(left.kind, ExprKind::Call { .. }) {
                    // Annex B permits a CallExpression as a sloppy assignment target.
                } else {
                    ensure_assignable(&left).map_err(|e| self.err(e.message))?;
                }
            } else {
                let annex_b_call = !self.strict_script
                    && !matches!(op, AssignOp::And | AssignOp::Or | AssignOp::Nullish);
                ensure_simple_assignable(&left, annex_b_call).map_err(|e| self.err(e.message))?;
            }
            if self.strict_script {
                if let Some(at) = strict_assignment_name(&left) {
                    return Err(self.err_at(at.start, "assignment to eval or arguments in strict mode"));
                }
            }
            self.bump(); // operator
            let value = self.parse_assignment_expr()?;
            Ok(Expr::new(
                ExprKind::Assign {
                    op,
                    target: Box::new(left),
                    value: Box::new(value),
                },
                self.span_from(start),
            ))
        } else {
            Ok(left)
        }
    }

    pub(super) fn parse_conditional_expr(&mut self) -> Result<Expr, SyntaxError> {
        let start = self.cur_loc();
        let test = self.parse_binary(MIN_PREC)?;
        if !self.at_punct(Punct::Question) {
            return Ok(test);
        }
        self.bump(); // ?
                     // The middle operand allows `in` even inside for-heads.
        let saved = self.no_in;
        self.no_in = false;
        let consequent = self.parse_assignment_expr();
        self.no_in = saved;
        let consequent = consequent?;
        self.expect_punct(Punct::Colon)?;
        let alternate = self.parse_assignment_expr()?;
        Ok(Expr::new(
            ExprKind::Conditional {
                test: Box::new(test),
                consequent: Box::new(consequent),
                alternate: Box::new(alternate),
            },
            self.span_from(start),
        ))
    }

    // ----- precedence climbing -----------------------------------------------

    /// Precedence-climbing loop over all binary and short-circuit operators.
    /// `saw_coalesce` / `saw_logical` enforce the spec rule that `??`
    /// cannot be mixed with `||`/`&&` without parentheses; the flags stay
    /// local to one unparenthesized chain because each *operand* recursion
    /// starts fresh.
    pub(super) fn parse_binary(&mut self, min_prec: u8) -> Result<Expr, SyntaxError> {
        let start = self.cur_loc();
        let mut left = self.parse_unary()?;
        let mut saw_coalesce = false;
        let mut saw_logical = false;
        loop {
            let (prec, kind) = match self.peek_binop() {
                Some(pair) => pair,
                None => break,
            };
            if prec < min_prec {
                break;
            }
            if kind == BinKind::Arithmetic(BinOp::In) && self.no_in {
                break;
            }
            if matches!(left.kind, ExprKind::PrivateName(_))
                && kind != BinKind::Arithmetic(BinOp::In)
            {
                return Err(self.err("private name is only valid with `in`"));
            }
            // Unary operators cannot directly feed `**` (the grammar gives
            // ExponentiationExpression no unary left operand), so
            // `-2 ** 2` is a SyntaxError while `(-2) ** 2` parses.
            if kind == BinKind::Arithmetic(BinOp::Pow)
                && matches!(left.kind, ExprKind::Unary { .. } | ExprKind::Update { .. })
            {
                return Err(self.err("unary operand needs parentheses before `**`"));
            }
            match kind {
                BinKind::Logical(LogicalOp::Coalesce) => {
                    if saw_logical {
                        return Err(
                            self.err("`??` cannot be mixed with `||`/`&&` without parentheses")
                        );
                    }
                    saw_coalesce = true;
                }
                BinKind::Logical(_) => {
                    if saw_coalesce {
                        return Err(
                            self.err("`||`/`&&` cannot be mixed with `??` without parentheses")
                        );
                    }
                    saw_logical = true;
                }
                BinKind::Arithmetic(_) => {}
            }
            self.bump(); // operator
                         // `**` is right-associative; everything else left-assoc.
            let next_min = if kind == BinKind::Arithmetic(BinOp::Pow) {
                prec
            } else {
                prec + 1
            };
            let right = self.parse_binary(next_min)?;
            if kind == BinKind::Arithmetic(BinOp::In)
                && matches!(left.kind, ExprKind::PrivateName(_))
                && matches!(right.kind, ExprKind::Arrow(_))
            {
                return Err(
                    self.err("private brand `in` requires a shift-expression right operand")
                );
            }
            // The flags above only see operators at the *current* precedence
            // level; `a ?? b || c` slips past them because the right-operand
            // recursion (min_prec 4) absorbs `||` silently. A logical op
            // absorbed there is always the root of the right subtree, so one
            // root check closes the hole. Parenthesized operands surface as
            // [`ExprKind::Paren`] and are correctly allowed.
            if kind == BinKind::Logical(LogicalOp::Coalesce)
                && matches!(
                    right.kind,
                    ExprKind::Logical {
                        op: LogicalOp::And | LogicalOp::Or,
                        ..
                    }
                )
            {
                return Err(self.err("`??` cannot be mixed with `||`/`&&` without parentheses"));
            }
            let span = Span::new(start, right.span.end);
            left = match kind {
                BinKind::Arithmetic(op) => Expr::new(
                    ExprKind::Binary {
                        op,
                        left: Box::new(left),
                        right: Box::new(right),
                    },
                    span,
                ),
                BinKind::Logical(op) => Expr::new(
                    ExprKind::Logical {
                        op,
                        left: Box::new(left),
                        right: Box::new(right),
                    },
                    span,
                ),
            };
        }
        if matches!(left.kind, ExprKind::PrivateName(_)) {
            return Err(self.err("private name is only valid with `in`"));
        }
        Ok(left)
    }

    pub(super) fn peek_binop(&self) -> Option<(u8, BinKind)> {
        use crate::tokens::Keyword as K;
        use BinKind::{Arithmetic as A, Logical as L};
        use BinOp::*;
        use LogicalOp as Lo;
        let pair = match self.cur().kind {
            TokenKind::Punct(Punct::Nullish) => (3, L(Lo::Coalesce)),
            TokenKind::Punct(Punct::Or) => (4, L(Lo::Or)),
            TokenKind::Punct(Punct::And) => (5, L(Lo::And)),
            TokenKind::Punct(Punct::BitOr) => (6, A(BitOr)),
            TokenKind::Punct(Punct::BitXor) => (7, A(BitXor)),
            TokenKind::Punct(Punct::BitAnd) => (8, A(BitAnd)),
            TokenKind::Punct(Punct::EqEq) => (9, A(EqEq)),
            TokenKind::Punct(Punct::NotEq) => (9, A(NotEq)),
            TokenKind::Punct(Punct::EqEqEq) => (9, A(StrictEq)),
            TokenKind::Punct(Punct::NotEqEq) => (9, A(StrictNeq)),
            TokenKind::Punct(Punct::Lt) => (10, A(Lt)),
            TokenKind::Punct(Punct::Gt) => (10, A(Gt)),
            TokenKind::Punct(Punct::Le) => (10, A(Le)),
            TokenKind::Punct(Punct::Ge) => (10, A(Ge)),
            TokenKind::Keyword(K::InstanceOf) => (10, A(InstanceOf)),
            TokenKind::Keyword(K::In) => (10, A(In)),
            TokenKind::Punct(Punct::Shl) => (11, A(Shl)),
            TokenKind::Punct(Punct::Shr) => (11, A(Shr)),
            TokenKind::Punct(Punct::UShr) => (11, A(UShr)),
            TokenKind::Punct(Punct::Plus) => (12, A(Add)),
            TokenKind::Punct(Punct::Minus) => (12, A(Sub)),
            TokenKind::Punct(Punct::Star) => (13, A(Mul)),
            TokenKind::Punct(Punct::Slash) => (13, A(Div)),
            TokenKind::Punct(Punct::Percent) => (13, A(Rem)),
            TokenKind::Punct(Punct::Pow) => (14, A(Pow)),
            _ => return None,
        };
        Some(pair)
    }

    // ----- unary & postfix -----------------------------------------------------

    pub(super) fn parse_unary(&mut self) -> Result<Expr, SyntaxError> {
        if self.cur().escaped_identifier && matches!(self.cur().kind, TokenKind::Keyword(_))
            && !(self.at_kw(crate::tokens::Keyword::Yield) && !self.strict_script && self.in_generator == 0) {
            return Err(self.err("escaped reserved word"));
        }
        let start = self.cur_loc();
        use crate::tokens::Keyword as K;
        // Prefix update.
        if self.at_punct(Punct::Inc) || self.at_punct(Punct::Dec) {
            let op = if self.eat_punct(Punct::Inc) {
                UpdateOp::Inc
            } else {
                self.bump();
                UpdateOp::Dec
            };
            let operand = self.parse_unary()?;
            ensure_simple_assignable(&operand, !self.strict_script).map_err(|e| self.err_at(start, e.message))?;
            if self.strict_script && strict_assignment_name(&operand).is_some() {
                return Err(self.err_at(start, "update of eval or arguments in strict mode"));
            }
            return Ok(Expr::new(
                ExprKind::Update {
                    op,
                    prefix: true,
                    target: Box::new(operand),
                },
                self.span_from(start),
            ));
        }
        let simple_unop = match self.cur().kind {
            TokenKind::Punct(Punct::Bang) => Some(UnOp::Not),
            TokenKind::Punct(Punct::Tilde) => Some(UnOp::BitNot),
            TokenKind::Punct(Punct::Plus) => Some(UnOp::Pos),
            TokenKind::Punct(Punct::Minus) => Some(UnOp::Neg),
            TokenKind::Keyword(K::Typeof) => Some(UnOp::Typeof),
            TokenKind::Keyword(K::Void) => Some(UnOp::Void),
            TokenKind::Keyword(K::Delete) => Some(UnOp::Delete),
            _ => None,
        };
        if let Some(op) = simple_unop {
            self.bump();
            if self.in_generator > 0 && matches!(self.cur().kind, TokenKind::Keyword(K::Yield)) {
                return Err(self.err("yield must be parenthesized in a unary operand"));
            }
            let operand = self.parse_unary()?;
            if op == UnOp::Delete && is_private_reference(&operand) {
                return Err(self.err_at(start, "cannot delete a private name"));
            }
            return Ok(Expr::new(
                ExprKind::Unary {
                    op,
                    operand: Box::new(operand),
                },
                self.span_from(start),
            ));
        }
        // `await` (contextual: only inside async functions).
        if self.word("await") && self.in_async > 0 {
            if self.in_formal_params > 0 {
                return Err(self.err("await is forbidden in async formal parameter defaults"));
            }
            self.bump();
            if self.module_goal && self.in_function == 0 {
                self.has_top_level_await = true;
            }
            let operand = self.parse_unary()?;
            return Ok(Expr::new(
                ExprKind::Unary {
                    op: UnOp::Await,
                    operand: Box::new(operand),
                },
                self.span_from(start),
            ));
        }
        self.parse_postfix()
    }

    /// Postfix `++`/`--` — postfix only when on the same line (ASI's
    /// restricted production; a newline ends the statement instead).
    pub(super) fn parse_postfix(&mut self) -> Result<Expr, SyntaxError> {
        let start = self.cur_loc();
        let target = self.parse_lhs()?;
        let is_update = matches!(self.cur().kind, TokenKind::Punct(Punct::Inc | Punct::Dec))
            && !self.cur().newline_before;
        if !is_update {
            return Ok(target);
        }
        ensure_simple_assignable(&target, !self.strict_script).map_err(|e| self.err(e.message))?;
        if self.strict_script && strict_assignment_name(&target).is_some() {
            return Err(self.err_at(start, "update of eval or arguments in strict mode"));
        }
        let op = if self.eat_punct(Punct::Inc) {
            UpdateOp::Inc
        } else {
            self.bump();
            UpdateOp::Dec
        };
        Ok(Expr::new(
            ExprKind::Update {
                op,
                prefix: false,
                target: Box::new(target),
            },
            self.span_from(start),
        ))
    }

    // ----- call / member / new chains --------------------------------------------

    pub(super) fn parse_lhs(&mut self) -> Result<Expr, SyntaxError> {
        let start = self.cur_loc();
        if self.at_kw(crate::tokens::Keyword::New) {
            if self.cur().escaped_identifier {
                return Err(self.err("escaped reserved word"));
            }
            self.bump(); // new
            if self.at_kw(crate::tokens::Keyword::Import)
                && (self.nth_punct(1, Punct::LParen)
                    || self.nth_punct(1, Punct::Dot)
                        && self.nth_punct(3, Punct::LParen)) {
                return Err(self.err_at(start, "dynamic import cannot be the operand of new"));
            }
            if self.eat_punct(Punct::Dot) {
                if !self.word("target") {
                    return Err(self.err("expected target after new."));
                }
                self.bump();
                if self.in_new_target == 0 && self.in_static_block == 0 {
                    return Err(self.err_at(start, "new.target is only valid in functions"));
                }
                return Ok(Expr::new(ExprKind::NewTarget, self.span_from(start)));
            }
            let callee = if self.at_kw(crate::tokens::Keyword::New) {
                self.parse_lhs()?
            } else {
                self.parse_member_chain(false)?
            };
            let args = if self.at_punct(Punct::LParen) {
                self.parse_args()?
            } else {
                Vec::new()
            };
            let node = Expr::new(
                ExprKind::New {
                    callee: Box::new(callee),
                    args,
                },
                self.span_from(start),
            );
            return self.parse_member_suffix(node, true);
        }
        self.parse_member_chain(true)
    }

    pub(super) fn parse_member_chain(&mut self, allow_calls: bool) -> Result<Expr, SyntaxError> {
        let prim = self.parse_primary()?;
        self.parse_member_suffix(prim, allow_calls)
    }

    pub(super) fn parse_member_suffix(&mut self, mut e: Expr, allow_calls: bool) -> Result<Expr, SyntaxError> {
        loop {
            if self.at_punct(Punct::Dot) {
                self.bump();
                if matches!(&e.kind, ExprKind::Super)
                    && matches!(self.cur().kind, TokenKind::PrivateIdent(_))
                {
                    return Err(self.err("super cannot access a private name"));
                }
                let prop = self.member_name()?;
                let span = Span::new(e.span.start, self.last_end);
                e = Expr::new(
                    ExprKind::Member {
                        obj: Box::new(e),
                        prop,
                        optional: false,
                    },
                    span,
                );
            } else if self.at_punct(Punct::LBracket) {
                self.bump();
                let idx = self.parse_expression()?;
                self.expect_punct(Punct::RBracket)?;
                let span = Span::new(e.span.start, self.last_end);
                e = Expr::new(
                    ExprKind::Member {
                        obj: Box::new(e),
                        prop: MemberProp::Computed(Box::new(idx)),
                        optional: false,
                    },
                    span,
                );
            } else if self.at_punct(Punct::QuestionDot) {
                self.bump();
                if self.at_punct(Punct::LParen) {
                    if !allow_calls {
                        break;
                    }
                    let args = self.parse_args()?;
                    let span = Span::new(e.span.start, self.last_end);
                    e = Expr::new(
                        ExprKind::Call {
                            callee: Box::new(e),
                            args,
                            optional: true,
                        },
                        span,
                    );
                } else if self.at_punct(Punct::LBracket) {
                    self.bump();
                    let idx = self.parse_expression()?;
                    self.expect_punct(Punct::RBracket)?;
                    let span = Span::new(e.span.start, self.last_end);
                    e = Expr::new(
                        ExprKind::Member {
                            obj: Box::new(e),
                            prop: MemberProp::Computed(Box::new(idx)),
                            optional: true,
                        },
                        span,
                    );
                } else {
                    if matches!(&e.kind, ExprKind::Super)
                        && matches!(self.cur().kind, TokenKind::PrivateIdent(_))
                    {
                        return Err(self.err("super cannot access a private name"));
                    }
                    let prop = self.member_name()?;
                    let span = Span::new(e.span.start, self.last_end);
                    e = Expr::new(
                        ExprKind::Member {
                            obj: Box::new(e),
                            prop,
                            optional: true,
                        },
                        span,
                    );
                }
            } else if self.at_punct(Punct::LParen) && allow_calls {
                if matches!(e.kind, ExprKind::Super) && !self.super_call_allowed {
                    return Err(self.err("super() is only valid in a class constructor"));
                }
                let args = self.parse_args()?;
                let span = Span::new(e.span.start, self.last_end);
                e = Expr::new(
                    ExprKind::Call {
                        callee: Box::new(e),
                        args,
                        optional: false,
                    },
                    span,
                );
            } else if matches!(self.cur().kind, TokenKind::TemplatePart { .. }) {
                if self.template_depth == 0 {
                    if has_unparenthesized_optional_chain(&e) {
                        return Err(self.err("optional chain cannot be a tagged template tag"));
                    }
                    let template = self.parse_template_lit()?;
                    let span = Span::new(e.span.start, self.last_end);
                    e = Expr::new(ExprKind::TaggedTemplate {
                        tag: Box::new(e),
                        template,
                    }, span);
                    continue;
                }
                // Continuation chunk of the enclosing `${ … }` (the lexer
                // absorbed its closing `}`); parse_template_lit consumes it.
                break;
            } else {
                break;
            }
        }
        Ok(e)
    }

    pub(super) fn parse_args(&mut self) -> Result<Vec<Expr>, SyntaxError> {
        self.expect_punct(Punct::LParen)?;
        let mut args = Vec::new();
        if !self.at_punct(Punct::RParen) {
            loop {
                let arg_start = self.cur_loc();
                if self.eat_punct(Punct::Ellipsis) {
                    let inner = self.parse_assignment_expr()?;
                    args.push(Expr::new(
                        ExprKind::Spread(Box::new(inner)),
                        self.span_from(arg_start),
                    ));
                } else {
                    args.push(self.parse_assignment_expr()?);
                }
                if !self.eat_punct(Punct::Comma) {
                    break;
                }
                if self.at_punct(Punct::RParen) {
                    break;
                }
            }
        }
        self.expect_punct(Punct::RParen)?;
        Ok(args)
    }

    // ----- primary ------------------------------------------------------------

    pub(super) fn parse_primary(&mut self) -> Result<Expr, SyntaxError> {
        let start = self.cur_loc();
        use crate::tokens::Keyword as K;
        if self.cur().escaped_identifier && matches!(self.cur().kind, TokenKind::Keyword(_))
            && !(self.at_kw(crate::tokens::Keyword::Yield) && !self.strict_script && self.in_generator == 0) {
            return Err(self.err("escaped reserved word"));
        }
        if self.at_punct(Punct::Slash) || self.at_punct(Punct::SlashEq) {
            self.tokens.borrow_mut().regex(self.pos)?;
        }
        match self.cur().kind.clone() {
            TokenKind::RegExp { pattern, flags } => {
                let compiled = amazing_regex::Regex::new(&pattern, &flags)
                    .map_err(|e| self.err(e.to_string()))?;
                self.bump();
                Ok(Expr::new(
                    ExprKind::RegExp(std::rc::Rc::new(compiled)),
                    self.span_from(start),
                ))
            }
            TokenKind::Numeric(n) => {
                if self.strict_script && n.radix == 10 && n.raw.starts_with('0')
                    && n.raw.len() > 1 && n.raw.as_bytes()[1].is_ascii_digit() {
                    return Err(self.err("legacy leading-zero literal in strict mode"));
                }
                self.bump();
                let kind = if n.bigint {
                    ExprKind::BigInt(bigint_decimal_digits(&n))
                } else {
                    ExprKind::Num(n)
                };
                Ok(Expr::new(kind, self.span_from(start)))
            }
            TokenKind::Str(s) => {
                let token = self.bump();
                let mut expr = Expr::new(ExprKind::Str(s), self.span_from(start));
                expr.string_utf16 = token.cooked_utf16;
                Ok(expr)
            }
            TokenKind::PrivateIdent(name) => {
                let t = self.bump();
                let id = Ident::new(name, t.span);
                self.use_private(id.clone())?;
                Ok(Expr::new(ExprKind::PrivateName(id), self.span_from(start)))
            }
            TokenKind::TemplatePart { .. } => {
                let lit = self.parse_template_lit()?;
                Ok(Expr::new(ExprKind::Template(lit), self.span_from(start)))
            }
            TokenKind::Keyword(K::This) => {
                self.bump();
                Ok(Expr::new(ExprKind::This, self.span_from(start)))
            }
            TokenKind::Keyword(K::Super) => {
                self.bump();
                if !self.at_punct(Punct::Dot) && !self.at_punct(Punct::LBracket)
                    && !self.at_punct(Punct::LParen) {
                    return Err(self.err_at(start, "bare super is not an expression"));
                }
                Ok(Expr::new(ExprKind::Super, self.span_from(start)))
            }
            TokenKind::Keyword(K::Null) => {
                self.bump();
                Ok(Expr::new(ExprKind::Null, self.span_from(start)))
            }
            TokenKind::Keyword(K::True) => {
                self.bump();
                Ok(Expr::new(ExprKind::Bool(true), self.span_from(start)))
            }
            TokenKind::Keyword(K::False) => {
                self.bump();
                Ok(Expr::new(ExprKind::Bool(false), self.span_from(start)))
            }
            TokenKind::Keyword(K::Function) => {
                let f = self.parse_function_rest(false)?;
                Ok(Expr::new(
                    ExprKind::Function(Box::new(f)),
                    self.span_from(start),
                ))
            }
            TokenKind::Keyword(K::Class) => {
                let c = self.parse_class_rest()?;
                Ok(Expr::new(
                    ExprKind::Class(Box::new(c)),
                    self.span_from(start),
                ))
            }
            TokenKind::Keyword(K::Import) if self.nth_punct(1, Punct::LParen) => {
                self.bump();
                let args = self.parse_args()?;
                if args.is_empty() || args.len() > 2 || args.iter().any(|arg| matches!(arg.kind, ExprKind::Spread(_))) {
                    return Err(self.err_at(start, "dynamic import requires one or two assignment expressions"));
                }
                Ok(Expr::new(ExprKind::DynamicImport(args), self.span_from(start)))
            }
            TokenKind::Keyword(K::Import) if self.nth_punct(1, Punct::Dot) => {
                if matches!(self.nth(2).kind, TokenKind::Ident(ref name) if name == "source" || name == "defer")
                    && self.nth_punct(3, Punct::LParen) {
                    self.bump(); // import
                    self.bump(); // .
                    let phase = match self.bump().kind { TokenKind::Ident(name) => name, _ => unreachable!() };
                    let args = self.parse_args()?;
                    if args.is_empty() || args.len() > 2 || args.iter().any(|arg| matches!(arg.kind, ExprKind::Spread(_))) {
                        return Err(self.err_at(start, "dynamic import requires one or two assignment expressions"));
                    }
                    return Ok(Expr::new(ExprKind::DynamicImportPhase { phase, args }, self.span_from(start)));
                }
                if !self.module_goal {
                    return Err(self.err("import.meta is only valid in modules"));
                }
                self.bump();
                self.bump();
                if !self.word("meta") {
                    return Err(self.err("expected import.meta"));
                }
                self.bump();
                Ok(Expr::new(ExprKind::ImportMeta, self.span_from(start)))
            }
            TokenKind::Keyword(K::Yield) if self.in_generator > 0 => self.parse_yield_expr(),
            TokenKind::Keyword(K::Yield) if !self.strict_script => {
                let token = self.bump();
                if self.at_punct(Punct::Arrow) && !self.cur().newline_before {
                    self.bump();
                    return self.finish_arrow(start, vec![plain_param(Ident::new("yield", token.span))], false);
                }
                Ok(Expr::new(ExprKind::Ident(Ident::new("yield", token.span)), self.span_from(start)))
            }
            TokenKind::Punct(Punct::LParen) => self.parse_paren_or_arrow(),
            TokenKind::Punct(Punct::LBracket) => self.parse_array_literal(),
            TokenKind::Punct(Punct::Lbrace) => self.parse_object_literal(),
            TokenKind::Ident(_) => self.parse_ident_primary(),
            other => Err(self.err(format!("unexpected {}", other.describe()))),
        }
    }

    /// Identifier-position primaries: plain references plus every arrow
    /// shape that starts with an identifier (`x =>`, `async x =>`,
    /// `async (…) =>`, `async function`).
    pub(super) fn parse_ident_primary(&mut self) -> Result<Expr, SyntaxError> {
        let start = self.cur_loc();
        // async function expression
        if self.word("async") && self.nth_is_function(1) {
            self.bump(); // async
            let f = self.parse_function_rest(true)?;
            return Ok(Expr::new(
                ExprKind::Function(Box::new(f)),
                self.span_from(start),
            ));
        }
        // async single-param arrow: `async x => …`
        if self.word("async")
            && matches!(self.nth(1).kind, TokenKind::Ident(_))
            && !self.nth(1).newline_before
            && self.nth_punct(2, Punct::Arrow)
            && !self.nth(2).newline_before
        {
            self.bump(); // async
            let param = self.expect_ident()?;
            self.bump(); // =>
            return self.finish_arrow(start, vec![plain_param(param)], true);
        }
        // async parenthesized arrow: speculate `(params) =>`.
        if self.word("async") && self.nth_punct(1, Punct::LParen) && !self.nth(1).newline_before {
            let save_pos = self.pos;
            let save_end = self.last_end;
            let save_noin = self.no_in;
            self.bump(); // async
            match self.try_arrow_params() {
                Ok(params) => return self.finish_arrow(start, params, true),
                Err(_) => {
                    self.pos = save_pos;
                    self.last_end = save_end;
                    self.no_in = save_noin;
                }
            }
        }
        // Single-param arrow fast path: `x => …` (covers `async => …` too,
        // where `async` is just the parameter name).
        if matches!(self.cur().kind, TokenKind::Ident(_))
            && self.nth_punct(1, Punct::Arrow)
            && !self.nth(1).newline_before
        {
            let param = self.expect_ident()?;
            self.bump(); // =>
            return self.finish_arrow(start, vec![plain_param(param)], false);
        }
        // Plain identifier reference.
        let id = self.expect_ident()?;
        if id.name == "arguments" && (self.in_class_field_initializer > 0 || self.in_static_block > 0) {
            return Err(self.err_at(
                id.span.start,
                "arguments is forbidden in a class field initializer",
            ));
        }
        Ok(Expr::new(ExprKind::Ident(id), self.span_from(start)))
    }

    /// The cover grammar for `(…)` : either parenthesized expressions or
    /// arrow parameters. Attempts arrow parameters strictly; rewinds to the
    /// saved position on any mismatch and parses parentheses instead.
    pub(super) fn parse_paren_or_arrow(&mut self) -> Result<Expr, SyntaxError> {
        let start = self.cur_loc();
        let save_pos = self.pos;
        let save_end = self.last_end;
        let save_noin = self.no_in;
        match self.try_arrow_params() {
            Ok(params) => return self.finish_arrow(start, params, false),
            Err(_) => {
                self.pos = save_pos;
                self.last_end = save_end;
                self.no_in = save_noin;
            }
        }
        self.expect_punct(Punct::LParen)?;
        if self.at_punct(Punct::RParen) {
            // Empty parens are only valid after a successful arrow attempt.
            return Err(self.err("empty parenthesized expression"));
        }
        let inner = self.parse_expression()?;
        self.expect_punct(Punct::RParen)?;
        Ok(Expr::new(
            ExprKind::Paren(Box::new(inner)),
            self.span_from(start),
        ))
    }

    /// Consumes `( binding (= default)? , … ) =>` or errors (caller
    /// rewinds). The Arrow token is consumed on success.
    pub(super) fn try_arrow_params(&mut self) -> Result<Vec<Param>, SyntaxError> {
        let params = self.parse_params()?;
        if !matches!(self.cur().kind, TokenKind::Punct(Punct::Arrow)) || self.cur().newline_before {
            return Err(self.err("not an arrow function"));
        }
        self.bump(); // =>
        Ok(params)
    }

    pub(super) fn finish_arrow(
        &mut self,
        start: SourceLocation,
        params: Vec<Param>,
        is_async: bool,
    ) -> Result<Expr, SyntaxError> {
        let body = self.with_fn_context(is_async, false, |p| p.parse_arrow_body())?;
        let own_strict = matches!(&body, ArrowBody::Block(block) if has_use_strict(block));
        if own_strict && params.iter().any(|param| param.rest || param.default.is_some()
            || !matches!(param.binding, Binding::Ident(_))) {
            return Err(self.err_at(start, "use strict with non-simple arrow parameters"));
        }
        let mut names = HashSet::new();
        for param in &params {
            for name in binding_names(&param.binding) {
                if !names.insert(name.name.clone()) {
                    return Err(self.err_at(name.span.start, "duplicate arrow parameter"));
                }
                if (self.strict_script || own_strict)
                    && matches!(name.name.as_str(), "eval" | "arguments") {
                    return Err(self.err_at(name.span.start, "invalid strict arrow parameter"));
                }
            }
        }
        Ok(Expr::new(
            ExprKind::Arrow(ArrowFunc {
                params,
                body,
                is_async,
                is_generator: false,
            }),
            self.span_from(start),
        ))
    }

    pub(super) fn parse_arrow_body(&mut self) -> Result<ArrowBody, SyntaxError> {
        if self.at_punct(Punct::Lbrace) {
            Ok(ArrowBody::Block(self.parse_block()?))
        } else {
            Ok(ArrowBody::Expr(Box::new(self.parse_assignment_expr()?)))
        }
    }

    pub(super) fn parse_yield_expr(&mut self) -> Result<Expr, SyntaxError> {
        let start = self.cur_loc();
        if self.in_formal_params > 0 {
            return Err(self.err("yield is forbidden in generator formal parameter defaults"));
        }
        if self.in_generator == 0 {
            return Err(self.err("`yield` is only valid inside generator functions"));
        }
        self.bump(); // yield
        let delegate = self.eat_punct(Punct::Star);
        let argument = if self.starts_expression() && !self.cur().newline_before {
            Some(Box::new(self.parse_assignment_expr()?))
        } else {
            None
        };
        Ok(Expr::new(
            ExprKind::Yield { argument, delegate },
            self.span_from(start),
        ))
    }

    // ----- literals -----------------------------------------------------------

    pub(super) fn parse_template_lit(&mut self) -> Result<TemplateLit, SyntaxError> {
        let t0 = self.bump();
        let first_utf16 = t0.cooked_utf16.unwrap_or_default();
        let first_raw_utf16 = t0.raw_utf16.unwrap_or_default();
        let (first, tail0) = match t0.kind {
            TokenKind::TemplatePart { cooked, tail } => (cooked, tail),
            _ => return Err(self.err("expected template literal")),
        };
        let mut quasis = vec![first];
        let mut quasis_utf16 = vec![first_utf16];
        let mut raw_quasis_utf16 = vec![first_raw_utf16];
        let mut exprs = Vec::new();
        let mut tail = tail0;
        while !tail {
            self.expect_punct(Punct::DollarBrace)?;
            self.template_depth += 1;
            let sub = self.parse_expression();
            self.template_depth -= 1;
            exprs.push(sub?);
            // The closing `}` was absorbed by the lexer together with the
            // next chunk, so a TemplatePart must follow directly.
            let t = self.bump();
            let cooked_utf16 = t.cooked_utf16.unwrap_or_default();
            let raw_utf16 = t.raw_utf16.unwrap_or_default();
            match t.kind {
                TokenKind::TemplatePart { cooked, tail: t2 } => {
                    quasis.push(cooked);
                    quasis_utf16.push(cooked_utf16);
                    raw_quasis_utf16.push(raw_utf16);
                    tail = t2;
                }
                other => {
                    return Err(self.err(format!(
                        "expected template continuation, found {}",
                        other.describe()
                    )));
                }
            }
        }
        Ok(TemplateLit {
            quasis,
            quasis_utf16,
            raw_quasis_utf16,
            exprs,
        })
    }

    pub(super) fn parse_array_literal(&mut self) -> Result<Expr, SyntaxError> {
        let start = self.cur_loc();
        self.expect_punct(Punct::LBracket)?;
        let mut elems: Vec<Option<Expr>> = Vec::new();
        let mut trailing_comma_after_spread = false;
        loop {
            if self.eat_punct(Punct::RBracket) {
                break;
            }
            if self.at_punct(Punct::Comma) {
                // Hole (or separator before another hole).
                self.bump();
                elems.push(None);
                continue;
            }
            let elem = if self.eat_punct(Punct::Ellipsis) {
                let inner = self.parse_assignment_with_in()?;
                Expr::new(ExprKind::Spread(Box::new(inner)), self.span_from(start))
            } else {
                self.parse_assignment_with_in()?
            };
            elems.push(Some(elem));
            if self.at_punct(Punct::Comma) {
                self.bump();
                if self.at_punct(Punct::RBracket) {
                    trailing_comma_after_spread = elems.last().and_then(|element| element.as_ref())
                        .is_some_and(|element| matches!(element.kind, ExprKind::Spread(_)));
                    self.bump(); // trailing comma, no hole
                    break;
                }
            } else {
                self.expect_punct(Punct::RBracket)?;
                break;
            }
        }
        let mut array = Expr::new(ExprKind::Array(elems), self.span_from(start));
        array.trailing_comma_after_spread = trailing_comma_after_spread;
        Ok(array)
    }

    pub(super) fn parse_object_literal(&mut self) -> Result<Expr, SyntaxError> {
        let start = self.cur_loc();
        self.expect_punct(Punct::Lbrace)?;
        let mut elems = Vec::new();
        let mut trailing_comma_after_spread = false;
        while !self.at_punct(Punct::Rbrace) {
            if self.at_eof() {
                return Err(self.err("unterminated object literal: expected `}`"));
            }
            elems.push(self.parse_object_element()?);
            if !self.eat_punct(Punct::Comma) {
                break;
            }
            if self.at_punct(Punct::Rbrace)
                && elems.last().is_some_and(|element| matches!(element, ObjElem::Spread(_))) {
                trailing_comma_after_spread = true;
            }
        }
        self.expect_punct(Punct::Rbrace)?;
        let mut object = Expr::new(ExprKind::Object(elems), self.span_from(start));
        object.trailing_comma_after_spread = trailing_comma_after_spread;
        Ok(object)
    }

    pub(super) fn parse_object_element(&mut self) -> Result<ObjElem, SyntaxError> {
        let start = self.cur_loc();
        if self.eat_punct(Punct::Ellipsis) {
            let inner = self.parse_assignment_with_in()?;
            return Ok(ObjElem::Spread(Box::new(Expr::new(
                ExprKind::Spread(Box::new(inner)),
                self.span_from(start),
            ))));
        }
        // Modifiers (no `static` in object literals).
        // Modifier order is fixed by the grammar: `async` precedes `*`
        // (`async *ag() {}`; bare `*g()` and `async m()` are the other combos).
        let is_async =
            self.word("async") && !self.nth(1).newline_before && self.nth_starts_member_key(1);
        if is_async {
            self.bump();
        }
        let is_generator = self.eat_punct(Punct::Star);
        let mut kind = MethodKind::Method;
        if !is_generator && !is_async {
            if self.word("get") && self.nth_starts_member_key(1) {
                self.bump();
                kind = MethodKind::Get;
            } else if self.word("set") && self.nth_starts_member_key(1) {
                self.bump();
                kind = MethodKind::Set;
            }
        }
        let key_start = self.cur_loc();
        let (key, key_utf16) = self.parse_prop_key()?;
        if self.at_punct(Punct::Colon) {
            self.bump();
            let value = self.parse_assignment_with_in()?;
            return Ok(ObjElem::Prop(Property {
                key,
                key_utf16,
                value: PropValue::Init(value),
                span: self.span_from(key_start),
            }));
        }
        if self.at_punct(Punct::LParen) {
            self.in_async += usize::from(is_async);
            self.in_generator += usize::from(is_generator);
            self.in_new_target += 1;
            let saved_static_block = self.in_static_block;
            self.in_static_block = 0;
            let params = self.parse_params();
            self.in_generator -= usize::from(is_generator);
            self.in_async -= usize::from(is_async);
            let params = match params {
                Ok(params) => params,
                Err(error) => {
                    self.in_new_target -= 1;
                    self.in_static_block = saved_static_block;
                    return Err(error);
                }
            };
            self.validate_accessor_arity(kind, &params, start)?;
            let body = self.with_fn_context(is_async, is_generator, |p| p.parse_block());
            self.in_new_target -= 1;
            self.in_static_block = saved_static_block;
            let body = body?;
            let own_strict_directive = has_use_strict(&body);
            self.validate_method_params(
                &params,
                self.strict_script || own_strict_directive,
                own_strict_directive,
            )?;
            return Ok(ObjElem::Prop(Property {
                key: key.clone(),
                key_utf16: key_utf16.clone(),
                value: PropValue::Method(ClassMethod {
                    key,
                    key_utf16,
                    kind,
                    is_static: false,
                    is_generator,
                    is_async,
                    params,
                    body,
                    span: self.span_from(start),
                }),
                span: self.span_from(start),
            }));
        }
        // Shorthand `{ a }` — only for unmodified identifier keys.
        if is_generator || is_async || kind != MethodKind::Method {
            return Err(self.err("method modifiers require a parameter list"));
        }
        match key {
            PropKey::Ident(ref id) => {
                if self.forbidden_binding_name(&id.name) {
                    return Err(self.err_at(id.span.start, "reserved shorthand property name"));
                }
                if self.eat_punct(Punct::Eq) {
                    let default = self.parse_assignment_with_in()?;
                    let binding = id.clone();
                    return Ok(ObjElem::Prop(Property {
                        key,
                        key_utf16,
                        value: PropValue::CoverInitializedName { binding, default },
                        span: self.span_from(start),
                    }));
                }
                let value = PropValue::Shorthand(Ident::new(id.name.clone(), id.span));
                Ok(ObjElem::Prop(Property {
                    key,
                    key_utf16,
                    value,
                    span: self.span_from(start),
                }))
            }
            _ => Err(self.err("expected `:` after property key")),
        }
    }
}

fn bigint_decimal_digits(n: &crate::tokens::Numeric) -> String {
    let mut digits = vec![0u8]; // little-endian base ten
    for ch in n.raw.chars() {
        let mut carry = ch.to_digit(n.radix as u32).expect("lexer validated BigInt digit");
        for digit in &mut digits {
            carry += u32::from(*digit) * u32::from(n.radix);
            *digit = (carry % 10) as u8;
            carry /= 10;
        }
        while carry > 0 {
            digits.push((carry % 10) as u8);
            carry /= 10;
        }
    }
    digits.iter().rev().map(|d| char::from(b'0' + d)).collect()
}

fn has_unparenthesized_optional_chain(expr: &Expr) -> bool {
    match &expr.kind {
        ExprKind::Member { obj, optional, .. } => *optional || has_unparenthesized_optional_chain(obj),
        ExprKind::Call { callee, optional, .. } => *optional || has_unparenthesized_optional_chain(callee),
        _ => false,
    }
}
