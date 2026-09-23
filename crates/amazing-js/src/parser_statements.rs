//! Statement, declaration, and binding parsing.
use super::*;

impl Parser {
    // ----- program & statements -------------------------------------------

    pub(super) fn parse_program(mut self) -> Result<Program, SyntaxError> {
        let start = self.cur_loc();
        let mut body = Vec::new();
        let mut in_directive_prologue = true;
        while !self.at_eof() {
            let stmt = self.parse_stmt()?;
            if in_directive_prologue {
                match directive_string(&stmt) {
                    Some("use strict") => self.strict_script = true,
                    Some(_) => {}
                    None => in_directive_prologue = false,
                }
            }
            body.push(stmt);
        }
        Ok(Program {
            span: self.span_from(start),
            body,
        })
    }

    pub(super) fn parse_stmt(&mut self) -> Result<Stmt, SyntaxError> {
        let start = self.cur_loc();
        let kind = match &self.cur().kind {
            TokenKind::Punct(Punct::Lbrace) => return self.parse_block_stmt(),
            TokenKind::Punct(Punct::Semi) => {
                self.bump();
                StmtKind::Empty
            }
            TokenKind::Keyword(crate::tokens::Keyword::Var) => {
                let decl = self.parse_var_decl(VarKind::Var, false)?;
                self.semicolon()?;
                StmtKind::Var(decl)
            }
            TokenKind::Keyword(crate::tokens::Keyword::Const) => {
                let decl = self.parse_var_decl(VarKind::Const, false)?;
                self.semicolon()?;
                StmtKind::Var(decl)
            }
            TokenKind::Ident(name) if name == "let" && self.let_starts_decl() => {
                let decl = self.parse_var_decl(VarKind::Let, false)?;
                self.semicolon()?;
                StmtKind::Var(decl)
            }
            TokenKind::Ident(name) if name == "async" && self.nth_is_function(1) => {
                self.bump(); // `async`
                let mut f = self.parse_function_rest(true)?;
                f.is_async = true;
                if f.name.is_none() {
                    return Err(self.err_at(start, "function declarations must have a name"));
                }
                StmtKind::Function(Box::new(f))
            }
            TokenKind::Keyword(crate::tokens::Keyword::Function) => {
                let f = self.parse_function_rest(false)?;
                if f.name.is_none() {
                    return Err(self.err_at(start, "function declarations must have a name"));
                }
                StmtKind::Function(Box::new(f))
            }
            TokenKind::Keyword(crate::tokens::Keyword::Class) => {
                let c = self.parse_class_rest()?;
                let name = c
                    .name
                    .clone()
                    .ok_or_else(|| self.err_at(start, "class declarations must have a name"))?;
                StmtKind::Class(Box::new(ClassItem {
                    name: Some(name),
                    ..c
                }))
            }
            TokenKind::Keyword(crate::tokens::Keyword::If) => self.parse_if_stmt()?,
            TokenKind::Keyword(crate::tokens::Keyword::For) => self.parse_for_stmt()?,
            TokenKind::Keyword(crate::tokens::Keyword::While) => self.parse_while_stmt()?,
            TokenKind::Keyword(crate::tokens::Keyword::Do) => self.parse_do_while_stmt()?,
            TokenKind::Keyword(crate::tokens::Keyword::Switch) => self.parse_switch_stmt()?,
            TokenKind::Keyword(crate::tokens::Keyword::Try) => self.parse_try_stmt()?,
            TokenKind::Keyword(crate::tokens::Keyword::Throw) => self.parse_throw_stmt()?,
            TokenKind::Keyword(crate::tokens::Keyword::Return) => self.parse_return_stmt()?,
            TokenKind::Keyword(crate::tokens::Keyword::Break) => self.parse_break_stmt()?,
            TokenKind::Keyword(crate::tokens::Keyword::Continue) => self.parse_continue_stmt()?,
            TokenKind::Keyword(crate::tokens::Keyword::Debugger) => {
                self.bump();
                self.semicolon()?;
                StmtKind::Debugger
            }
            TokenKind::Keyword(crate::tokens::Keyword::Import)
            | TokenKind::Keyword(crate::tokens::Keyword::Export) => {
                return Err(self.err("modules (import/export) are deferred"));
            }
            TokenKind::Ident(name) if name == "import" || name == "export" => {
                return Err(self.err("modules (import/export) are deferred"));
            }
            _ => {
                // Labeled statement: `name :` (colon on any line? Spec
                // requires the colon to follow; keep it simple and accept).
                if matches!(self.cur().kind, TokenKind::Ident(_)) && self.nth_punct(1, Punct::Colon)
                {
                    if self.word("await") && self.in_async > 0 {
                        return Err(self.err("await cannot be a label in an async function"));
                    }
                    return self.parse_labeled_stmt();
                }
                let expr = self.parse_expression_no_in(false)?;
                self.semicolon()?;
                StmtKind::Expression(expr)
            }
        };
        Ok(Stmt::new(kind, self.span_from(start)))
    }

    /// `let` begins a declaration only when followed by a binding start.
    /// `let[0]` and `let.x` stay expressions (destructuring deferred → the
    /// bracket/brace forms get a dedicated diagnostic).
    pub(super) fn let_starts_decl(&self) -> bool {
        matches!(self.nth(1).kind, TokenKind::Ident(_))
            || matches!(self.nth(1).kind, TokenKind::Punct(Punct::Lbrace))
            || matches!(self.nth(1).kind, TokenKind::Punct(Punct::LBracket))
    }

    pub(super) fn nth_is_function(&self, n: usize) -> bool {
        matches!(
            self.nth(n).kind,
            TokenKind::Keyword(crate::tokens::Keyword::Function)
        ) && !self.nth(n).newline_before
    }

    /// Parses one `var`/`let`/`const` declarator list. `allow_missing_init`
    /// relaxes the const-requires-initializer rule for for-heads, where the
    /// deciding context (`in`/`of` vs `;`) is only known afterwards; the
    /// C-style path re-checks explicitly.
    pub(super) fn parse_var_decl(
        &mut self,
        kind: VarKind,
        allow_missing_init: bool,
    ) -> Result<VarDecl, SyntaxError> {
        self.bump(); // var/let/const keyword
        let mut decls = Vec::new();
        loop {
            let dstart = self.cur_loc();
            let binding = self.parse_binding()?;
            let init = if self.eat_punct(Punct::Eq) {
                Some(self.parse_assignment_expr()?)
            } else {
                None
            };
            if init.is_none() && kind == VarKind::Const && !allow_missing_init {
                return Err(self.err_at(dstart, "const declarations require an initializer"));
            }
            decls.push(Declarator {
                binding,
                init,
                span: self.span_from(dstart),
            });
            if !self.eat_punct(Punct::Comma) {
                break;
            }
        }
        Ok(VarDecl { kind, decls })
    }

    pub(super) fn parse_binding(&mut self) -> Result<Binding, SyntaxError> {
        self.parse_binding_inner(false)
    }

    pub(super) fn parse_binding_inner(&mut self, allow_default: bool) -> Result<Binding, SyntaxError> {
        let mut binding = match self.cur().kind {
            TokenKind::Punct(Punct::LBracket) => {
                self.bump();
                let mut elements = Vec::new();
                while !self.at_punct(Punct::RBracket) {
                    if self.eat_punct(Punct::Comma) {
                        elements.push(None);
                        continue;
                    }
                    if self.eat_punct(Punct::Ellipsis) {
                        elements.push(Some(Box::new(Binding::Rest(Box::new(
                            self.parse_binding_inner(false)?,
                        )))));
                        if self.eat_punct(Punct::Comma) && !self.at_punct(Punct::RBracket) {
                            return Err(self.err("rest element must be last in an array pattern"));
                        }
                        break;
                    }
                    elements.push(Some(Box::new(self.parse_binding_inner(true)?)));
                    if !self.eat_punct(Punct::Comma) {
                        break;
                    }
                }
                self.expect_punct(Punct::RBracket)?;
                Binding::Array(elements)
            }
            TokenKind::Punct(Punct::Lbrace) => {
                self.bump();
                let mut properties = Vec::new();
                while !self.at_punct(Punct::Rbrace) {
                    if self.eat_punct(Punct::Ellipsis) {
                        let rest = self.parse_binding_inner(false)?;
                        properties.push((
                            BindingKey::Static("...".to_string()),
                            Box::new(Binding::Rest(Box::new(rest))),
                        ));
                        if self.eat_punct(Punct::Comma) && !self.at_punct(Punct::Rbrace) {
                            return Err(self.err("rest property must be last in an object pattern"));
                        }
                        break;
                    }
                    let (key, shorthand_ident) = if self.eat_punct(Punct::LBracket) {
                        let expression = self.parse_assignment_expr()?;
                        self.expect_punct(Punct::RBracket)?;
                        (BindingKey::Computed(Box::new(expression)), None)
                    } else {
                        let identifier = self.prop_name()?;
                        (
                            BindingKey::Static(identifier.name.clone()),
                            Some(identifier),
                        )
                    };
                    let value = if self.eat_punct(Punct::Colon) {
                        self.parse_binding_inner(true)?
                    } else if let Some(identifier) = shorthand_ident {
                        let mut shorthand = Binding::Ident(identifier);
                        if self.eat_punct(Punct::Eq) {
                            let saved = self.no_in;
                            self.no_in = true;
                            let default = self.parse_assignment_expr()?;
                            self.no_in = saved;
                            shorthand = Binding::Default(Box::new(shorthand), Box::new(default));
                        }
                        shorthand
                    } else {
                        return Err(self.err("computed binding keys require a target"));
                    };
                    properties.push((key, Box::new(value)));
                    if !self.eat_punct(Punct::Comma) {
                        break;
                    }
                }
                self.expect_punct(Punct::Rbrace)?;
                Binding::Object(properties)
            }
            _ => Binding::Ident(self.expect_ident()?),
        };
        if allow_default && self.eat_punct(Punct::Eq) {
            let saved = self.no_in;
            self.no_in = true;
            let default = self.parse_assignment_expr()?;
            self.no_in = saved;
            binding = Binding::Default(Box::new(binding), Box::new(default));
        }
        Ok(binding)
    }

    pub(super) fn parse_block_stmt(&mut self) -> Result<Stmt, SyntaxError> {
        let block = self.parse_block()?;
        let span = block.span;
        Ok(Stmt::new(StmtKind::Block(block), span))
    }

    pub(super) fn parse_block(&mut self) -> Result<Block, SyntaxError> {
        let start = self.expect_punct(Punct::Lbrace)?;
        let mut body = Vec::new();
        while !self.at_punct(Punct::Rbrace) {
            if self.at_eof() {
                return Err(self.err("unterminated block: expected `}`"));
            }
            body.push(self.parse_stmt()?);
        }
        self.bump(); // }
        Ok(Block {
            body,
            span: self.span_from(start.start),
        })
    }

    pub(super) fn parse_if_stmt(&mut self) -> Result<StmtKind, SyntaxError> {
        self.bump(); // if
        self.expect_punct(Punct::LParen)?;
        let cond = self.parse_expression_no_in(false)?;
        self.expect_punct(Punct::RParen)?;
        let then = Box::new(self.parse_stmt()?);
        let els = if self.eat_kw(crate::tokens::Keyword::Else) {
            Some(Box::new(self.parse_stmt()?))
        } else {
            None
        };
        Ok(StmtKind::If { cond, then, els })
    }

    pub(super) fn parse_while_stmt(&mut self) -> Result<StmtKind, SyntaxError> {
        self.bump(); // while
        self.expect_punct(Punct::LParen)?;
        let test = self.parse_expression_no_in(false)?;
        self.expect_punct(Punct::RParen)?;
        self.loop_depth += 1;
        let body = Box::new(self.parse_stmt()?);
        self.loop_depth -= 1;
        Ok(StmtKind::While { test, body })
    }

    pub(super) fn parse_do_while_stmt(&mut self) -> Result<StmtKind, SyntaxError> {
        self.bump(); // do
        self.loop_depth += 1;
        let body = Box::new(self.parse_stmt()?);
        self.loop_depth -= 1;
        if !self.eat_kw(crate::tokens::Keyword::While) {
            return Err(self.err("expected `while` after do-while body"));
        }
        self.expect_punct(Punct::LParen)?;
        let test = self.parse_expression_no_in(false)?;
        self.expect_punct(Punct::RParen)?;
        // `do … while (…) ` is followed by optional `;` (ASI rule 1).
        self.semicolon()?;
        Ok(StmtKind::DoWhile { body, test })
    }

    pub(super) fn parse_for_stmt(&mut self) -> Result<StmtKind, SyntaxError> {
        self.bump(); // for
        self.expect_punct(Punct::LParen)?;
        // Empty init: C-style only.
        if self.at_punct(Punct::Semi) {
            self.bump(); // the `;` terminating the (absent) init clause
            return self.parse_for_c_style(None);
        }
        // Lexical declaration head.
        let decl_head = match &self.cur().kind {
            TokenKind::Keyword(crate::tokens::Keyword::Var) => Some(VarKind::Var),
            TokenKind::Keyword(crate::tokens::Keyword::Const) => Some(VarKind::Const),
            TokenKind::Ident(name) if name == "let" && self.let_starts_decl() => Some(VarKind::Let),
            _ => None,
        };
        if let Some(kind) = decl_head {
            let kw_start = self.cur_loc();
            // Missing initializers are allowed here because a for-in/of head
            // must not have one; the C-style tail below re-imposes the rule.
            let decl = self.parse_var_decl_no_in(kind)?;
            // for-in / for-of?
            if self.at_kw(crate::tokens::Keyword::In) || self.word("of") {
                let decl = validate_for_decl(&decl, kw_start)?;
                let stmt = if self.word("of") {
                    self.bump();
                    let right = self.parse_assignment_expr()?;
                    self.expect_punct(Punct::RParen)?;
                    self.loop_depth += 1;
                    let body = Box::new(self.parse_stmt()?);
                    self.loop_depth -= 1;
                    StmtKind::ForOf {
                        left: ForLeft::Decl(Box::new(decl)),
                        right,
                        body,
                    }
                } else {
                    self.bump(); // in
                    let right = self.parse_expression_no_in(true)?;
                    self.expect_punct(Punct::RParen)?;
                    self.loop_depth += 1;
                    let body = Box::new(self.parse_stmt()?);
                    self.loop_depth -= 1;
                    StmtKind::ForIn {
                        left: ForLeft::Decl(Box::new(decl)),
                        right,
                        body,
                    }
                };
                return Ok(stmt);
            }
            self.expect_punct(Punct::Semi)?;
            if kind == VarKind::Const && decl.decls.iter().any(|d| d.init.is_none()) {
                return Err(self.err_at(kw_start, "const declarations require an initializer"));
            }
            return self.parse_for_c_style(Some(ForInit::Decl(Box::new(decl))));
        }
        // Expression head: `for (x in y)` / `for (x of y)` / C-style.
        let expr = self.parse_expression_no_in(true)?;
        if self.at_kw(crate::tokens::Keyword::In) || self.word("of") {
            ensure_assignable(&expr).map_err(|e| self.err_at(self.cur_loc(), e.message))?;
            let stmt = if self.word("of") {
                self.bump();
                let right = self.parse_assignment_expr()?;
                self.expect_punct(Punct::RParen)?;
                self.loop_depth += 1;
                let body = Box::new(self.parse_stmt()?);
                self.loop_depth -= 1;
                StmtKind::ForOf {
                    left: ForLeft::Target(expr),
                    right,
                    body,
                }
            } else {
                self.bump(); // in
                let right = self.parse_expression_no_in(true)?;
                self.expect_punct(Punct::RParen)?;
                self.loop_depth += 1;
                let body = Box::new(self.parse_stmt()?);
                self.loop_depth -= 1;
                StmtKind::ForIn {
                    left: ForLeft::Target(expr),
                    right,
                    body,
                }
            };
            return Ok(stmt);
        }
        self.expect_punct(Punct::Semi)?;
        self.parse_for_c_style(Some(ForInit::Expr(expr)))
    }

    /// C-style tail after `init;`: `cond; update) body`.
    pub(super) fn parse_for_c_style(&mut self, init: Option<ForInit>) -> Result<StmtKind, SyntaxError> {
        // Caller consumed the `;` after init when init exists.
        let test = if self.at_punct(Punct::Semi) {
            None
        } else {
            Some(self.parse_expression_no_in(true)?)
        };
        self.expect_punct(Punct::Semi)?;
        let update = if self.at_punct(Punct::RParen) {
            None
        } else {
            Some(self.parse_expression_no_in(true)?)
        };
        self.expect_punct(Punct::RParen)?;
        self.loop_depth += 1;
        let body = Box::new(self.parse_stmt()?);
        self.loop_depth -= 1;
        Ok(StmtKind::For {
            init,
            test,
            update,
            body,
        })
    }

    /// Like [`Parser::parse_var_decl`] but parses initializers with the
    /// `no-in` restriction active (for-heads) and missing initializers
    /// permitted (the for-in/of head re-checks).
    pub(super) fn parse_var_decl_no_in(&mut self, kind: VarKind) -> Result<VarDecl, SyntaxError> {
        let saved = self.no_in;
        self.no_in = true;
        let r = self.parse_var_decl(kind, true);
        self.no_in = saved;
        r
    }

    pub(super) fn parse_switch_stmt(&mut self) -> Result<StmtKind, SyntaxError> {
        self.bump(); // switch
        self.expect_punct(Punct::LParen)?;
        let disc = self.parse_expression_no_in(false)?;
        self.expect_punct(Punct::RParen)?;
        self.expect_punct(Punct::Lbrace)?;
        let mut cases = Vec::new();
        let mut seen_default = false;
        self.switch_depth += 1;
        while !self.at_punct(Punct::Rbrace) {
            if self.at_eof() {
                return Err(self.err("unterminated switch: expected `}`"));
            }
            let cstart = self.cur_loc();
            let test = if self.eat_kw(crate::tokens::Keyword::Case) {
                let e = self.parse_expression_no_in(false)?;
                Some(e)
            } else if self.eat_kw(crate::tokens::Keyword::Default) {
                if seen_default {
                    return Err(self.err("a switch may have only one `default` clause"));
                }
                seen_default = true;
                None
            } else {
                return Err(self.err("expected `case` or `default` in switch body"));
            };
            self.expect_punct(Punct::Colon)?;
            let mut body = Vec::new();
            while !self.at_punct(Punct::Rbrace)
                && !self.at_kw(crate::tokens::Keyword::Case)
                && !self.at_kw(crate::tokens::Keyword::Default)
            {
                body.push(self.parse_stmt()?);
            }
            cases.push(SwitchCase {
                test,
                body,
                span: self.span_from(cstart),
            });
        }
        self.switch_depth -= 1;
        self.bump(); // }
        Ok(StmtKind::Switch { disc, cases })
    }

    pub(super) fn parse_try_stmt(&mut self) -> Result<StmtKind, SyntaxError> {
        self.bump(); // try
        let block = self.parse_block()?;
        let mut catch = None;
        let mut finally = None;
        if self.eat_kw(crate::tokens::Keyword::Catch) {
            let param = if self.eat_punct(Punct::LParen) {
                let p = self.expect_ident()?;
                self.expect_punct(Punct::RParen)?;
                Some(p)
            } else {
                None
            };
            let body = self.parse_block()?;
            catch = Some(CatchClause { param, body });
        }
        if self.eat_kw(crate::tokens::Keyword::Finally) {
            finally = Some(self.parse_block()?);
        }
        if catch.is_none() && finally.is_none() {
            return Err(self.err("try statement requires `catch` or `finally`"));
        }
        Ok(StmtKind::Try(TryStmt {
            block,
            catch,
            finally,
        }))
    }

    pub(super) fn parse_throw_stmt(&mut self) -> Result<StmtKind, SyntaxError> {
        self.bump(); // throw
        if self.cur().newline_before {
            return Err(self.err("newline not allowed after `throw`"));
        }
        let expr = self.parse_expression_no_in(false)?;
        self.semicolon()?;
        Ok(StmtKind::Throw(expr))
    }

    pub(super) fn parse_return_stmt(&mut self) -> Result<StmtKind, SyntaxError> {
        self.bump(); // return
        if self.in_function == 0 {
            return Err(self.err("`return` outside of a function"));
        }
        let arg = if self.starts_expression() && !self.cur().newline_before {
            Some(self.parse_expression_no_in(false)?)
        } else {
            None
        };
        self.semicolon()?;
        Ok(StmtKind::Return(arg))
    }

    pub(super) fn parse_break_stmt(&mut self) -> Result<StmtKind, SyntaxError> {
        self.bump(); // break
        let label = self.opt_label_ref()?;
        match &label {
            Some(id) => {
                if !self.labels.iter().any(|l| l.name == id.name) {
                    return Err(self.err(format!("undefined label `{}`", id.name)));
                }
            }
            None => {
                if self.loop_depth == 0 && self.switch_depth == 0 {
                    return Err(self.err("`break` not inside a loop or switch"));
                }
            }
        }
        self.semicolon()?;
        Ok(StmtKind::Break { label })
    }

    pub(super) fn parse_continue_stmt(&mut self) -> Result<StmtKind, SyntaxError> {
        self.bump(); // continue
        let label = self.opt_label_ref()?;
        match &label {
            Some(id) => {
                if !self.labels.iter().any(|l| l.name == id.name) {
                    return Err(self.err(format!("undefined label `{}`", id.name)));
                }
                if !self.labels.iter().any(|l| l.name == id.name && l.is_loop) {
                    return Err(self.err(format!(
                        "`continue` target `{}` is not a loop label",
                        id.name
                    )));
                }
            }
            None => {
                if self.loop_depth == 0 {
                    return Err(self.err("`continue` not inside a loop"));
                }
            }
        }
        self.semicolon()?;
        Ok(StmtKind::Continue { label })
    }

    /// Label reference after break/continue: an identifier on the same line.
    pub(super) fn opt_label_ref(&mut self) -> Result<Option<Ident>, SyntaxError> {
        if matches!(self.cur().kind, TokenKind::Ident(_)) && !self.cur().newline_before {
            Ok(Some(self.expect_ident()?))
        } else {
            Ok(None)
        }
    }

    pub(super) fn parse_labeled_stmt(&mut self) -> Result<Stmt, SyntaxError> {
        let label = self.expect_ident()?;
        self.expect_punct(Punct::Colon)?;
        // Loop-ness must be known *before* the body parses (so an innermost
        // `continue outer:` resolves), and it holds through chains of labels
        // (`outer: inner: for …`). Decide it by probing tokens: skip any
        // further `ident :` pairs and see whether a loop keyword follows.
        let mut probe = 0usize;
        let is_loop = loop {
            if matches!(self.nth(probe).kind, TokenKind::Ident(_))
                && self.nth_punct(probe + 1, Punct::Colon)
            {
                probe += 2;
            } else {
                break matches!(
                    self.nth(probe).kind,
                    TokenKind::Keyword(
                        crate::tokens::Keyword::For
                            | crate::tokens::Keyword::While
                            | crate::tokens::Keyword::Do
                    )
                );
            }
        };
        self.labels.push(LabelCtx {
            name: label.name.clone(),
            is_loop,
        });
        let body = self.parse_stmt()?;
        let stmt = Stmt::new(
            StmtKind::Labeled {
                label: label.clone(),
                body: Box::new(body),
            },
            label.span,
        );
        self.labels.pop();
        Ok(stmt)
    }

    /// Does the current token plausibly start an expression? Used by the
    /// restricted-production checks (`return`/`yield` operand suppression).
    pub(super) fn starts_expression(&self) -> bool {
        match &self.cur().kind {
            TokenKind::Punct(Punct::Slash | Punct::SlashEq)
            | TokenKind::RegExp { .. }
            | TokenKind::Numeric(_)
            | TokenKind::Str(_)
            | TokenKind::TemplatePart { .. }
            | TokenKind::Ident(_)
            | TokenKind::PrivateIdent(_)
            | TokenKind::Punct(
                Punct::LParen
                | Punct::LBracket
                | Punct::Lbrace
                | Punct::Bang
                | Punct::Tilde
                | Punct::Plus
                | Punct::Minus
                | Punct::Inc
                | Punct::Dec,
            ) => true,
            TokenKind::Keyword(k) => matches!(
                k,
                crate::tokens::Keyword::This
                    | crate::tokens::Keyword::Null
                    | crate::tokens::Keyword::True
                    | crate::tokens::Keyword::False
                    | crate::tokens::Keyword::New
                    | crate::tokens::Keyword::Function
                    | crate::tokens::Keyword::Class
                    | crate::tokens::Keyword::Super
                    | crate::tokens::Keyword::Typeof
                    | crate::tokens::Keyword::Void
                    | crate::tokens::Keyword::Delete
                    | crate::tokens::Keyword::Yield
            ),
            _ => false,
        }
    }

}
