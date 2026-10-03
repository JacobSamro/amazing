//! Statement, declaration, and binding parsing.
use super::*;

impl Parser {
    // ----- program & statements -------------------------------------------

    pub(super) fn parse_program(&mut self) -> Result<Program, SyntaxError> {
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
        self.validate_scope_names(&body, false)?;
        if self.strict_script {
            self.validate_strict_statements(&body)?;
            if let Some(at) = self.tokens.borrow().legacy_escape_in(self.span_from(start)) {
                return Err(self.err_at(at, "legacy string escape is forbidden in strict mode"));
            }
            if let Some(at) = self.tokens.borrow().legacy_numeric_in(self.span_from(start)) {
                return Err(self.err_at(at, "legacy leading-zero literal in strict mode"));
            }
        }
        cover::validate_program(&body)?;
        Ok(Program {
            span: self.span_from(start),
            body,
        })
    }

    pub(super) fn parse_stmt(&mut self) -> Result<Stmt, SyntaxError> {
        let start = self.cur_loc();
        if self.word("let") && self.nth_punct(1, Punct::LBracket)
            && self.nth(1).newline_before {
            return Err(self.err("expression statements cannot begin with `let [`"));
        }
        if self.cur().escaped_identifier && matches!(self.cur().kind, TokenKind::Keyword(_))
            && !(matches!(self.cur().kind, TokenKind::Keyword(crate::tokens::Keyword::Yield)) && !self.strict_script && self.in_generator == 0) {
            return Err(self.err("escaped reserved word"));
        }
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
            TokenKind::Ident(name) if name == "using" && self.word("using")
                && matches!(self.nth(1).kind, TokenKind::Ident(_))
                && !self.nth(1).newline_before => {
                let decl = self.parse_using_decl(false, false)?;
                self.semicolon()?;
                StmtKind::Var(decl)
            }
            TokenKind::Ident(name) if name == "await" && self.word("await")
                && matches!(self.nth(1).kind, TokenKind::Ident(ref next) if next == "using")
                && !self.nth(1).newline_before && !self.nth(2).newline_before
                && matches!(self.nth(2).kind, TokenKind::Ident(_)) => {
                let decl = self.parse_using_decl(true, false)?;
                self.semicolon()?;
                StmtKind::Var(decl)
            }
            TokenKind::Ident(name) if name == "let" && self.word("let") && self.let_starts_decl() => {
                let decl = self.parse_var_decl(VarKind::Let, false)?;
                self.semicolon()?;
                StmtKind::Var(decl)
            }
            TokenKind::Ident(name) if name == "async" && self.word("async") && self.nth_is_function(1) => {
                self.bump(); // `async`
                let mut f = self.parse_function_rest(true, true)?;
                f.is_async = true;
                if f.name.is_none() {
                    return Err(self.err_at(start, "function declarations must have a name"));
                }
                StmtKind::Function(Box::new(f))
            }
            TokenKind::Keyword(crate::tokens::Keyword::Function) => {
                let f = self.parse_function_rest(false, true)?;
                if f.name.is_none() {
                    return Err(self.err_at(start, "function declarations must have a name"));
                }
                StmtKind::Function(Box::new(f))
            }
            TokenKind::Keyword(crate::tokens::Keyword::Class) | TokenKind::Punct(Punct::At) => {
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
            TokenKind::Keyword(crate::tokens::Keyword::With) => {
                if self.strict_script {
                    return Err(self.err("with statement is forbidden in strict mode"));
                }
                self.bump();
                self.expect_punct(Punct::LParen)?;
                let object = self.parse_expression()?;
                self.expect_punct(Punct::RParen)?;
                let body = self.parse_statement_position()?;
                if matches!(&body.kind, StmtKind::Var(decl) if decl.kind != VarKind::Var)
                    || matches!(body.kind, StmtKind::Function(_) | StmtKind::Class(_))
                    || is_labeled_function(&body) {
                    return Err(self.err_at(body.span.start, "with body cannot be a declaration"));
                }
                let body = Box::new(body);
                StmtKind::With { object, body }
            }
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
                if self.nth_punct(1, Punct::LParen) || self.nth_punct(1, Punct::Dot) => {
                let expr = self.parse_expression_no_in(false)?;
                self.semicolon()?;
                StmtKind::Expression(expr)
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
                if (matches!(self.cur().kind, TokenKind::Ident(_))
                    || matches!(self.cur().kind, TokenKind::Keyword(crate::tokens::Keyword::Yield)) && !self.strict_script && self.in_generator == 0)
                    && self.nth_punct(1, Punct::Colon)
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

    /// A single-statement body uses Statement grammar. In sloppy code, `let`
    /// followed by a line break can be an identifier expression before the
    /// next statement, even when that statement begins with a binding token.
    fn parse_statement_position(&mut self) -> Result<Stmt, SyntaxError> {
        if !self.strict_script && !self.module_goal && self.word("let")
            && self.nth(1).newline_before
            && (matches!(self.nth(1).kind, TokenKind::Ident(_))
                || self.nth_punct(1, Punct::Lbrace))
        {
            let start = self.cur_loc();
            let expression = self.parse_expression_no_in(false)?;
            self.semicolon()?;
            return Ok(Stmt::new(StmtKind::Expression(expression), self.span_from(start)));
        }
        self.parse_stmt()
    }

    /// `let` begins a declaration only when followed by a binding start.
    /// `let[0]` and `let.x` stay expressions (destructuring deferred → the
    /// bracket/brace forms get a dedicated diagnostic).
    pub(super) fn let_starts_decl(&self) -> bool {
        matches!(self.nth(1).kind, TokenKind::Ident(_))
            || matches!(self.nth(1).kind, TokenKind::Keyword(crate::tokens::Keyword::Yield))
            || matches!(self.nth(1).kind, TokenKind::Punct(Punct::Lbrace))
            || matches!(self.nth(1).kind, TokenKind::Punct(Punct::LBracket))
                && !self.nth(1).newline_before
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
            if kind != VarKind::Var {
                let mut seen = HashSet::new();
                for name in binding_names(&binding) {
                    if !seen.insert(&name.name) {
                        return Err(self.err_at(name.span.start, "duplicate lexical binding"));
                    }
                }
            }
            if kind != VarKind::Var && binding_names(&binding).iter().any(|name| name.name == "let") {
                return Err(self.err_at(dstart, "let cannot be a lexical binding name"));
            }
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
        if kind != VarKind::Var {
            let mut seen = HashSet::new();
            for name in decls.iter().flat_map(|decl| binding_names(&decl.binding)) {
                if !seen.insert(&name.name) {
                    return Err(self.err_at(name.span.start, "duplicate lexical binding"));
                }
            }
        }
        Ok(VarDecl { kind, decls, resource_kind: None })
    }

    fn parse_using_decl(&mut self, await_using: bool, for_head: bool) -> Result<VarDecl, SyntaxError> {
        if !for_head && !self.module_goal && self.block_depth == 0 {
            return Err(self.err("using declarations require a block"));
        }
        if await_using {
            if self.in_async == 0 { return Err(self.err("await using requires async context")); }
            self.bump(); // await
            if self.module_goal && self.in_function == 0 { self.has_top_level_await = true; }
        }
        self.bump(); // using
        let mut decls = Vec::new();
        loop {
            let start = self.cur_loc();
            if self.cur().newline_before { return Err(self.err("line break after using")); }
            let binding = Binding::Ident(self.expect_ident()?);
            if matches!(&binding, Binding::Ident(name) if name.name == "let") {
                return Err(self.err_at(start, "let cannot be a using binding"));
            }
            let init = if self.eat_punct(Punct::Eq) { Some(self.parse_assignment_expr()?) } else { None };
            if init.is_none() && !for_head { return Err(self.err_at(start, "using requires an initializer")); }
            decls.push(Declarator { binding, init, span: self.span_from(start) });
            if !self.eat_punct(Punct::Comma) { break; }
        }
        Ok(VarDecl {
            kind: VarKind::Const,
            decls,
            resource_kind: Some(if await_using { ResourceKind::AwaitUsing } else { ResourceKind::Using }),
        })
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
                        let saved_no_in = self.no_in;
                        self.no_in = false;
                        let expression = self.parse_assignment_expr();
                        self.no_in = saved_no_in;
                        let expression = expression?;
                        self.expect_punct(Punct::RBracket)?;
                        (BindingKey::Computed(Box::new(expression)), None)
                    } else if let TokenKind::Str(value) = self.cur().kind.clone() {
                        self.bump();
                        (BindingKey::Static(value), None)
                    } else if matches!(self.cur().kind, TokenKind::Numeric(_)) {
                        // A numeric literal's raw digits are not its property name
                        // (radix prefixes, exponents and BigInt all need NumericValue).
                        // Retain the constant literal through the existing key path;
                        // no user code runs when converting its primitive value.
                        let literal = self.parse_primary()?;
                        (BindingKey::Computed(Box::new(literal)), None)
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
                        if self.forbidden_binding_name(&identifier.name) {
                            return Err(self.err_at(identifier.span.start, "reserved shorthand binding name"));
                        }
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
        self.validate_scope_names(&block.body, true)?;
        let span = block.span;
        Ok(Stmt::new(StmtKind::Block(block), span))
    }

    pub(super) fn parse_function_block(&mut self) -> Result<Block, SyntaxError> {
        let saved_strict = self.strict_script;
        let result = self.parse_block_contents(true);
        self.strict_script = saved_strict;
        let block = result?;
        if saved_strict || has_use_strict(&block) {
            if let Some(at) = self.tokens.borrow().legacy_escape_in(block.span) {
                return Err(self.err_at(at, "legacy string escape is forbidden in strict mode"));
            }
            if let Some(at) = self.tokens.borrow().legacy_numeric_in(block.span) {
                return Err(self.err_at(at, "legacy leading-zero literal in strict mode"));
            }
        }
        Ok(block)
    }

    pub(super) fn parse_block(&mut self) -> Result<Block, SyntaxError> {
        self.parse_block_contents(false)
    }

    fn parse_block_contents(&mut self, mut directives: bool) -> Result<Block, SyntaxError> {
        let start = self.expect_punct(Punct::Lbrace)?;
        self.block_depth += 1;
        let mut body = Vec::new();
        while !self.at_punct(Punct::Rbrace) {
            if self.at_eof() {
                return Err(self.err("unterminated block: expected `}`"));
            }
            let statement = self.parse_stmt()?;
            if directives {
                match directive_string(&statement) {
                    Some("use strict") => self.strict_script = true,
                    Some(_) => {},
                    None => directives = false,
                }
            }
            body.push(statement);
        }
        self.bump(); // }
        self.block_depth -= 1;
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
        let then = Box::new(self.parse_statement_position()?);
        reject_statement_position_declaration(&then, self.strict_script, true)?;
        let els = if self.eat_kw(crate::tokens::Keyword::Else) {
            let statement = Box::new(self.parse_statement_position()?);
            reject_statement_position_declaration(&statement, self.strict_script, true)?;
            Some(statement)
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
        let body = Box::new(self.parse_statement_position()?);
        reject_statement_position_declaration(&body, true, true)?;
        self.loop_depth -= 1;
        Ok(StmtKind::While { test, body })
    }

    pub(super) fn parse_do_while_stmt(&mut self) -> Result<StmtKind, SyntaxError> {
        self.bump(); // do
        self.loop_depth += 1;
        let body = Box::new(self.parse_statement_position()?);
        reject_statement_position_declaration(&body, true, true)?;
        self.loop_depth -= 1;
        if !self.eat_kw(crate::tokens::Keyword::While) {
            return Err(self.err("expected `while` after do-while body"));
        }
        self.expect_punct(Punct::LParen)?;
        let test = self.parse_expression_no_in(false)?;
        self.expect_punct(Punct::RParen)?;
        // The do-while closing semicolon is optional even when the next
        // statement starts on the same line (ASI rule 1 special case).
        self.eat_punct(Punct::Semi);
        Ok(StmtKind::DoWhile { body, test })
    }

    pub(super) fn parse_for_stmt(&mut self) -> Result<StmtKind, SyntaxError> {
        self.bump(); // for
        let for_await = self.word("await");
        if for_await {
            if self.in_async == 0 { return Err(self.err("for await requires async context")); }
            self.bump();
            if self.module_goal && self.in_function == 0 { self.has_top_level_await = true; }
        }
        self.expect_punct(Punct::LParen)?;
        let await_using = self.word("await")
            && matches!(self.nth(1).kind, TokenKind::Ident(ref word) if word == "using")
            && !self.nth(1).newline_before;
        if (self.word("using")
            && (!matches!(self.nth(1).kind, TokenKind::Ident(ref name) if name == "of")
                || self.nth_punct(2, Punct::Eq))) || await_using {
            let decl = self.parse_using_decl(await_using, true)?;
            if self.word("of") {
                let decl = validate_for_decl(&decl, self.cur_loc(), false)?;
                self.bump();
                let right = self.parse_assignment_expr()?;
                self.expect_punct(Punct::RParen)?;
                self.loop_depth += 1;
                let body = Box::new(self.parse_statement_position()?);
                reject_statement_position_declaration(&body, true, true)?;
                self.loop_depth -= 1;
                validate_for_head_body(&decl, &body, self.cur_loc())?;
                return Ok(if for_await { StmtKind::ForAwaitOf { left: ForLeft::Decl(Box::new(decl)), right, body } }
                    else { StmtKind::ForOf { left: ForLeft::Decl(Box::new(decl)), right, body } });
            }
            if for_await { return Err(self.err("for await requires an of clause")); }
            if decl.decls.iter().any(|item| item.init.is_none()) {
                return Err(self.err("for using requires an initializer"));
            }
            self.expect_punct(Punct::Semi)?;
            return self.parse_for_c_style(Some(ForInit::Decl(Box::new(decl))));
        }
        // Empty init: C-style only.
        if self.at_punct(Punct::Semi) {
            if for_await { return Err(self.err("for await requires an of clause")); }
            self.bump(); // the `;` terminating the (absent) init clause
            return self.parse_for_c_style(None);
        }
        // Lexical declaration head.
        let decl_head = match &self.cur().kind {
            TokenKind::Keyword(crate::tokens::Keyword::Var) => Some(VarKind::Var),
            TokenKind::Keyword(crate::tokens::Keyword::Const) => Some(VarKind::Const),
            TokenKind::Ident(name) if name == "let" && self.word("let") && self.let_starts_decl() => Some(VarKind::Let),
            _ => None,
        };
        if let Some(kind) = decl_head {
            let kw_start = self.cur_loc();
            // Missing initializers are allowed here because a for-in/of head
            // must not have one; the C-style tail below re-imposes the rule.
            let decl = self.parse_var_decl_no_in(kind)?;
            // for-in / for-of?
            if self.at_kw(crate::tokens::Keyword::In) || self.word("of") {
                let annex_b_for_in = self.at_kw(crate::tokens::Keyword::In) && !self.strict_script;
                let decl = validate_for_decl(&decl, kw_start, annex_b_for_in)?;
                let stmt = if self.word("of") {
                    self.bump();
                    let right = self.parse_assignment_expr()?;
                    self.expect_punct(Punct::RParen)?;
                    self.loop_depth += 1;
                    let body = Box::new(self.parse_statement_position()?);
                    reject_statement_position_declaration(&body, true, true)?;
                    self.loop_depth -= 1;
                    validate_for_head_body(&decl, &body, kw_start)?;
                    if for_await { StmtKind::ForAwaitOf {
                        left: ForLeft::Decl(Box::new(decl)), right, body,
                    } } else { StmtKind::ForOf {
                        left: ForLeft::Decl(Box::new(decl)),
                        right,
                        body,
                    } }
                } else {
                    if for_await { return Err(self.err("for await requires an of clause")); }
                    self.bump(); // in
                    let right = self.parse_expression_no_in(true)?;
                    self.expect_punct(Punct::RParen)?;
                    self.loop_depth += 1;
                    let body = Box::new(self.parse_statement_position()?);
                    reject_statement_position_declaration(&body, true, true)?;
                    self.loop_depth -= 1;
                    validate_for_head_body(&decl, &body, kw_start)?;
                    StmtKind::ForIn {
                        left: ForLeft::Decl(Box::new(decl)),
                        right,
                        body,
                    }
                };
                return Ok(stmt);
            }
            self.expect_punct(Punct::Semi)?;
            if for_await { return Err(self.err("for await requires an of clause")); }
            if kind == VarKind::Const && decl.decls.iter().any(|d| d.init.is_none()) {
                return Err(self.err_at(kw_start, "const declarations require an initializer"));
            }
            return self.parse_for_c_style(Some(ForInit::Decl(Box::new(decl))));
        }
        // Expression head: `for (x in y)` / `for (x of y)` / C-style.
        let unescaped_async_head = self.word("async");
        let expr = self.parse_expression_no_in(true)?;
        if self.at_kw(crate::tokens::Keyword::In) || self.word("of") {
            if !for_await && unescaped_async_head && self.word("of")
                && matches!(&expr.kind, ExprKind::Ident(id) if id.name == "async") {
                return Err(self.err_at(expr.span.start, "async cannot be a for-of assignment target"));
            }
            if matches!(expr.kind, ExprKind::Assign { .. }) {
                return Err(self.err_at(expr.span.start, "for-in/of assignment head needs a target"));
            }
            if self.strict_script && strict_assignment_name(&expr).is_some() {
                return Err(self.err_at(expr.span.start, "assignment to eval or arguments in strict mode"));
            }
            let sloppy_annex_b_call = !self.strict_script
                && matches!(expr.kind, ExprKind::Call { .. });
            if !sloppy_annex_b_call {
                ensure_assignable(&expr).map_err(|e| self.err_at(self.cur_loc(), e.message))?;
            }
            let stmt = if self.word("of") {
                self.bump();
                let right = self.parse_assignment_expr()?;
                self.expect_punct(Punct::RParen)?;
                self.loop_depth += 1;
                let body = Box::new(self.parse_statement_position()?);
                reject_statement_position_declaration(&body, true, true)?;
                self.loop_depth -= 1;
                if for_await { StmtKind::ForAwaitOf {
                    left: ForLeft::Target(expr), right, body,
                } } else { StmtKind::ForOf {
                    left: ForLeft::Target(expr),
                    right,
                    body,
                } }
            } else {
                if for_await { return Err(self.err("for await requires an of clause")); }
                self.bump(); // in
                let right = self.parse_expression_no_in(true)?;
                self.expect_punct(Punct::RParen)?;
                self.loop_depth += 1;
                let body = Box::new(self.parse_statement_position()?);
                reject_statement_position_declaration(&body, true, true)?;
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
        if for_await { return Err(self.err("for await requires an of clause")); }
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
        let body = Box::new(self.parse_statement_position()?);
        reject_statement_position_declaration(&body, true, true)?;
        self.loop_depth -= 1;
        if let Some(ForInit::Decl(decl)) = &init {
            validate_for_head_body(decl, &body, body.span.start)?;
        }
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
                if body.last().is_some_and(is_resource_statement) {
                    return Err(self.err("using declaration is not allowed in switch case"));
                }
            }
            cases.push(SwitchCase {
                test,
                body,
                span: self.span_from(cstart),
            });
        }
        self.switch_depth -= 1;
        let switch_body: Vec<Stmt> = cases.iter().flat_map(|case| case.body.iter().cloned()).collect();
        self.validate_scope_names(&switch_body, true)?;
        self.bump(); // }
        Ok(StmtKind::Switch { disc, cases })
    }

    pub(super) fn validate_scope_names(&self, body: &[Stmt], block_functions: bool) -> Result<(), SyntaxError> {
        let mut lexical = HashSet::new();
        let mut var: HashSet<String> = body.iter()
            .filter(|statement| !matches!(statement.kind, StmtKind::Var(_)))
            .flat_map(var_declared_names)
            .collect();
        let mut annex_b_functions = HashSet::new();
        for statement in body {
            let mut names = Vec::new();
            let is_lexical = match &statement.kind {
                StmtKind::Var(decl) => {
                    for declarator in &decl.decls {
                        names.extend(binding_names(&declarator.binding));
                    }
                    decl.kind != VarKind::Var
                }
                StmtKind::Class(class) => {
                    names.extend(class.name.iter());
                    true
                }
                StmtKind::Function(function) if block_functions => {
                    names.extend(function.name.iter());
                    true
                }
                StmtKind::Function(function) => {
                    names.extend(function.name.iter());
                    false
                }
                _ => continue,
            };
            let annex_b_function = block_functions && !self.strict_script
                && matches!(&statement.kind, StmtKind::Function(function)
                    if !function.is_async && !function.is_generator);
            for name in names {
                if is_lexical {
                    let previous = !lexical.insert(name.name.clone());
                    if (previous && !(annex_b_function && annex_b_functions.contains(&name.name)))
                        || var.contains(&name.name) {
                        return Err(self.err_at(name.span.start, "duplicate lexical binding"));
                    }
                    if annex_b_function {
                        annex_b_functions.insert(name.name.clone());
                    }
                } else {
                    if lexical.contains(&name.name) {
                        return Err(self.err_at(name.span.start, "var conflicts with lexical binding"));
                    }
                    var.insert(name.name.clone());
                }
            }
        }
        Ok(())
    }

    pub(super) fn validate_strict_statements(&self, body: &[Stmt]) -> Result<(), SyntaxError> {
        for statement in body {
            match &statement.kind {
                StmtKind::With { .. } => {
                    return Err(self.err_at(statement.span.start, "with statement is forbidden in strict mode"));
                }
                StmtKind::Var(decl) => {
                    for item in &decl.decls {
                        for name in binding_names(&item.binding) {
                            if strict_binding_name(&name.name) {
                                return Err(self.err_at(name.span.start, "invalid strict binding name"));
                            }
                        }
                        if let Some(init) = &item.init { self.validate_strict_expression(init)?; }
                    }
                }
                StmtKind::Expression(expr) => self.validate_strict_expression(expr)?,
                StmtKind::Block(block) => self.validate_strict_statements(&block.body)?,
                StmtKind::Function(function) => {
                    if let Some(name) = &function.name {
                        if strict_binding_name(&name.name) {
                            return Err(self.err_at(name.span.start, "invalid strict function name"));
                        }
                    }
                    self.validate_strict_statements(&function.body.body)?;
                }
                StmtKind::If { then, els, .. } => {
                    self.validate_strict_statements(std::slice::from_ref(then.as_ref()))?;
                    if let Some(els) = els {
                        self.validate_strict_statements(std::slice::from_ref(els.as_ref()))?;
                    }
                }
                StmtKind::For { init, body, .. } => {
                    if let Some(ForInit::Decl(decl)) = init {
                        self.validate_strict_for_decl(decl)?;
                    }
                    self.validate_strict_statements(std::slice::from_ref(body.as_ref()))?;
                }
                StmtKind::ForIn { left, body, .. } | StmtKind::ForOf { left, body, .. }
                | StmtKind::ForAwaitOf { left, body, .. } => {
                    if let ForLeft::Decl(decl) = left {
                        self.validate_strict_for_decl(decl)?;
                    }
                    self.validate_strict_statements(std::slice::from_ref(body.as_ref()))?;
                }
                StmtKind::While { body, .. } | StmtKind::DoWhile { body, .. }
                | StmtKind::Labeled { body, .. } => {
                    self.validate_strict_statements(std::slice::from_ref(body.as_ref()))?;
                }
                StmtKind::Try(try_stmt) => {
                    self.validate_strict_statements(&try_stmt.block.body)?;
                    if let Some(catch) = &try_stmt.catch {
                        if let Some(binding) = &catch.binding {
                            if let Some(name) = binding_names(binding).into_iter()
                                .find(|name| strict_binding_name(&name.name)) {
                                return Err(self.err_at(name.span.start, "invalid strict catch binding"));
                            }
                        }
                        self.validate_strict_statements(&catch.body.body)?;
                    }
                    if let Some(finally) = &try_stmt.finally {
                        self.validate_strict_statements(&finally.body)?;
                    }
                }
                StmtKind::Switch { cases, .. } => {
                    for case in cases {
                        self.validate_strict_statements(&case.body)?;
                    }
                }
                _ => {}
            }
        }
        Ok(())
    }

    fn validate_strict_for_decl(&self, decl: &VarDecl) -> Result<(), SyntaxError> {
        for name in decl.decls.iter().flat_map(|item| binding_names(&item.binding)) {
            if strict_binding_name(&name.name) {
                return Err(self.err_at(name.span.start, "invalid strict loop binding name"));
            }
        }
        Ok(())
    }

    fn validate_strict_expression(&self, expr: &Expr) -> Result<(), SyntaxError> {
        match &expr.kind {
            ExprKind::Assign { target, .. } | ExprKind::Update { target, .. }
                if strict_assignment_name(target).is_some() => {
                return Err(self.err_at(expr.span.start, "assignment to eval or arguments in strict mode"));
            }
            ExprKind::Object(elements) => {
                for element in elements {
                    match element {
                        ObjElem::Spread(value) => self.validate_strict_expression(value)?,
                        ObjElem::Prop(property) => match &property.value {
                            PropValue::Shorthand(id) if matches!(id.name.as_str(),
                                "implements" | "interface" | "let" | "package" | "private"
                                | "protected" | "public" | "static" | "yield") => {
                                return Err(self.err_at(id.span.start, "strict reserved shorthand"));
                            }
                            PropValue::Init(value) => self.validate_strict_expression(value)?,
                            _ => {}
                        },
                    }
                }
            }
            ExprKind::Paren(inner) | ExprKind::Spread(inner) => self.validate_strict_expression(inner)?,
            ExprKind::Unary { op: UnOp::Delete, operand }
                if is_identifier_reference(operand) => {
                return Err(self.err_at(expr.span.start, "cannot delete an identifier in strict mode"));
            }
            ExprKind::Sequence(items) => {
                for item in items { self.validate_strict_expression(item)?; }
            }
            ExprKind::Assign { target, value, .. } | ExprKind::Binary { left: target, right: value, .. }
            | ExprKind::Logical { left: target, right: value, .. } => {
                self.validate_strict_expression(target)?;
                self.validate_strict_expression(value)?;
            }
            ExprKind::Call { callee, args, .. } | ExprKind::New { callee, args } => {
                self.validate_strict_expression(callee)?;
                for arg in args { self.validate_strict_expression(arg)?; }
            }
            ExprKind::Function(function) => self.validate_strict_statements(&function.body.body)?,
            ExprKind::Arrow(arrow) => match &arrow.body {
                ArrowBody::Expr(expr) => self.validate_strict_expression(expr)?,
                ArrowBody::Block(block) => self.validate_strict_statements(&block.body)?,
            },
            _ => {}
        }
        Ok(())
    }

    pub(super) fn parse_try_stmt(&mut self) -> Result<StmtKind, SyntaxError> {
        self.bump(); // try
        let block = self.parse_block()?;
        let mut catch = None;
        let mut finally = None;
        if self.eat_kw(crate::tokens::Keyword::Catch) {
            let binding = if self.eat_punct(Punct::LParen) {
                let p = self.parse_binding()?;
                self.expect_punct(Punct::RParen)?;
                let mut names = HashSet::new();
                for name in binding_names(&p) {
                    if !names.insert(name.name.clone()) {
                        return Err(self.err_at(name.span.start, "duplicate catch binding"));
                    }
                }
                Some(p)
            } else {
                None
            };
            let param = match &binding {
                Some(Binding::Ident(id)) => Some(id.clone()),
                _ => None,
            };
            let body = self.parse_block()?;
            if let Some(binding) = &binding {
                let names: HashSet<_> = binding_names(binding).into_iter()
                    .map(|name| name.name.as_str()).collect();
                for statement in &body.body {
                    let lexical = match &statement.kind {
                        StmtKind::Var(decl) if decl.kind != VarKind::Var => decl.decls.iter()
                            .flat_map(|item| binding_names(&item.binding)).collect::<Vec<_>>(),
                        StmtKind::Class(class) => class.name.iter().collect(),
                        StmtKind::Function(function) => function.name.iter().collect(),
                        _ => Vec::new(),
                    };
                    if let Some(name) = lexical.into_iter().find(|name| names.contains(name.name.as_str())) {
                        return Err(self.err_at(name.span.start, "catch binding conflicts with lexical declaration"));
                    }
                }
            }
            catch = Some(CatchClause { param, binding, body });
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
        if (matches!(self.cur().kind, TokenKind::Ident(_))
            || matches!(self.cur().kind, TokenKind::Keyword(crate::tokens::Keyword::Yield))
                && !self.strict_script && self.in_generator == 0) && !self.cur().newline_before {
            Ok(Some(self.expect_ident()?))
        } else {
            Ok(None)
        }
    }

    pub(super) fn parse_labeled_stmt(&mut self) -> Result<Stmt, SyntaxError> {
        let label = self.expect_ident()?;
        if self.labels.iter().any(|active| active.name == label.name) {
            return Err(self.err_at(label.span.start, "duplicate label"));
        }
        self.expect_punct(Punct::Colon)?;
        // Loop-ness must be known *before* the body parses (so an innermost
        // `continue outer:` resolves), and it holds through chains of labels
        // (`outer: inner: for …`). Decide it by probing tokens: skip any
        // further `ident :` pairs and see whether a loop keyword follows.
        let mut probe = 0usize;
        let is_loop = loop {
            if (matches!(self.nth(probe).kind, TokenKind::Ident(_))
                || matches!(self.nth(probe).kind, TokenKind::Keyword(crate::tokens::Keyword::Yield))
                    && !self.strict_script && self.in_generator == 0)
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
        let body = self.parse_statement_position()?;
        reject_statement_position_declaration(&body, self.strict_script, false)?;
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
        if self.cur().template_continuation { return false; }
        match &self.cur().kind {
            TokenKind::Punct(Punct::Slash | Punct::SlashEq)
            | TokenKind::RegExp { .. }
            | TokenKind::Numeric(_)
            | TokenKind::Str(_)
            | TokenKind::TemplatePart { .. }
            | TokenKind::Ident(_)
            | TokenKind::PrivateIdent(_)
            | TokenKind::Punct(
                Punct::At
                | Punct::LParen
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
                    | crate::tokens::Keyword::Import
            ),
            _ => false,
        }
    }

}

fn is_labeled_function(statement: &Stmt) -> bool {
    match &statement.kind {
        StmtKind::Function(_) => true,
        StmtKind::Labeled { body, .. } => is_labeled_function(body),
        _ => false,
    }
}

fn is_resource_statement(statement: &Stmt) -> bool {
    matches!(&statement.kind, StmtKind::Var(decl) if decl.resource_kind.is_some())
}

fn strict_binding_name(name: &str) -> bool {
    matches!(name, "eval" | "arguments" | "implements" | "interface" | "let" | "package"
        | "private" | "protected" | "public" | "static" | "yield")
}

fn reject_statement_position_declaration(statement: &Stmt, strict: bool, reject_labeled_function: bool) -> Result<(), SyntaxError> {
    let forbidden = match &statement.kind {
        StmtKind::Var(decl) => decl.kind != VarKind::Var,
        StmtKind::Class(_) => true,
        StmtKind::Function(function) => strict || function.is_async || function.is_generator,
        StmtKind::Labeled { .. } => reject_labeled_function && is_labeled_function(statement),
        _ => false,
    };
    if forbidden {
        Err(SyntaxError::new("declaration is not allowed in statement position", statement.span.start.line, statement.span.start.column))
    } else { Ok(()) }
}

fn validate_for_head_body(decl: &VarDecl, body: &Stmt, at: SourceLocation) -> Result<(), SyntaxError> {
    if decl.kind == VarKind::Var { return Ok(()); }
    let head: HashSet<String> = decl.decls.iter()
        .flat_map(|item| binding_names(&item.binding).into_iter().map(|name| name.name.clone()))
        .collect();
    if var_declared_names(body).iter().any(|name| head.contains(name)) {
        return Err(SyntaxError::new("for declaration conflicts with var in body", at.line, at.column));
    }
    Ok(())
}

fn var_declared_names(statement: &Stmt) -> Vec<String> {
    match &statement.kind {
        StmtKind::Var(decl) if decl.kind == VarKind::Var => decl.decls.iter()
            .flat_map(|item| binding_names(&item.binding).into_iter().map(|name| name.name.clone())).collect(),
        StmtKind::Block(block) => block.body.iter().flat_map(var_declared_names).collect(),
        StmtKind::If { then, els, .. } => var_declared_names(then).into_iter()
            .chain(els.iter().flat_map(|statement| var_declared_names(statement))).collect(),
        StmtKind::While { body, .. } | StmtKind::DoWhile { body, .. }
        | StmtKind::For { body, .. } | StmtKind::ForIn { body, .. }
        | StmtKind::ForOf { body, .. } | StmtKind::ForAwaitOf { body, .. }
        | StmtKind::Labeled { body, .. } | StmtKind::With { body, .. } => var_declared_names(body),
        StmtKind::Switch { cases, .. } => cases.iter().flat_map(|case| case.body.iter().flat_map(var_declared_names)).collect(),
        StmtKind::Try(try_stmt) => try_stmt.block.body.iter().flat_map(var_declared_names)
            .chain(try_stmt.catch.iter().flat_map(|catch| catch.body.body.iter().flat_map(var_declared_names)))
            .chain(try_stmt.finally.iter().flat_map(|block| block.body.iter().flat_map(var_declared_names))).collect(),
        _ => Vec::new(),
    }
}
