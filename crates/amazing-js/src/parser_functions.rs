//! Function, class, parameter, and property-key parsing.
use super::*;

impl Parser {
    // ----- functions -------------------------------------------------------

    /// Parses everything after `async`?/`function` — the keyword itself is
    /// consumed here. Callers fix up `is_async`.
    pub(super) fn parse_function_rest(&mut self, is_async: bool) -> Result<FunctionItem, SyntaxError> {
        self.bump(); // function
        let is_generator = self.eat_punct(Punct::Star);
        let name = if matches!(self.cur().kind, TokenKind::Keyword(crate::tokens::Keyword::Yield))
            && !self.strict_script && !is_generator {
            let token = self.bump();
            Some(Ident::new("yield", token.span))
        } else if matches!(self.cur().kind, TokenKind::Ident(_)) {
            Some(self.expect_ident()?)
        } else {
            None
        };
        let saved_async = self.in_async;
        let saved_generator = self.in_generator;
        let saved_static_block = self.in_static_block;
        self.in_static_block = 0;
        self.in_async = usize::from(is_async);
        self.in_generator = usize::from(is_generator);
        self.in_new_target += 1;
        let params = self.parse_params();
        if params.is_err() {
            self.in_async = saved_async;
            self.in_generator = saved_generator;
            self.in_new_target -= 1;
            self.in_static_block = saved_static_block;
        }
        let params = params?;
        self.in_function += 1;
        let saved_field = self.in_class_field_initializer;
        let saved_super = self.super_call_allowed;
        let saved_super_property = self.super_property_allowed;
        let saved_formals = self.in_formal_params;
        self.in_class_field_initializer = 0;
        self.super_call_allowed = false;
        self.super_property_allowed = false;
        self.in_formal_params = 0;
        let body = self.parse_block();
        self.in_formal_params = saved_formals;
        self.super_call_allowed = saved_super;
        self.super_property_allowed = saved_super_property;
        self.in_class_field_initializer = saved_field;
        self.in_async = saved_async;
        self.in_generator = saved_generator;
        self.in_function -= 1;
        self.in_new_target -= 1;
        self.in_static_block = saved_static_block;
        let body = body?;
        if let Some(name) = &name {
            if (self.strict_script || has_use_strict(&body)) && matches!(name.name.as_str(), "eval" | "arguments" | "yield") {
                return Err(self.err_at(name.span.start, "invalid strict function name"));
            }
        }
        if self.strict_script || has_use_strict(&body) {
            if let Some(at) = self.tokens.borrow().legacy_escape_in(body.span) {
                return Err(self.err_at(at, "legacy string escape is forbidden in strict mode"));
            }
        }
        self.validate_scope_names(&body.body, false)?;
        self.validate_function_params(&params, &body, is_async || is_generator)?;
        if has_use_strict(&body) {
            self.validate_strict_statements(&body.body)?;
        }
        Ok(FunctionItem {
            name,
            params,
            body,
            is_async,
            is_generator,
        })
    }

    pub(super) fn validate_function_params(
        &self,
        params: &[Param],
        body: &Block,
        force_unique: bool,
    ) -> Result<(), SyntaxError> {
        let strict = self.strict_script || has_use_strict(body);
        let simple = params.iter().all(|param| !param.rest && param.default.is_none()
            && matches!(param.binding, Binding::Ident(_)));
        if !simple && has_use_strict(body) {
            return Err(self.err_at(body.span.start, "use strict directive with non-simple parameters"));
        }
        let mut seen = HashSet::new();
        for param in params {
            for name in binding_names(&param.binding) {
                if (strict || force_unique || !simple) && !seen.insert(name.name.clone()) {
                    return Err(self.err_at(name.span.start, "duplicate parameter"));
                }
                if strict && matches!(name.name.as_str(), "eval" | "arguments" | "implements" | "interface" | "package" | "private" | "protected" | "public" | "static" | "yield" | "let") {
                    return Err(self.err_at(name.span.start, "invalid strict parameter name"));
                }
            }
        }
        let parameter_names: HashSet<_> = params.iter().flat_map(|param| binding_names(&param.binding))
            .map(|name| name.name.clone()).collect();
        for statement in &body.body {
            let lexical = match &statement.kind {
                StmtKind::Var(decl) if decl.kind != VarKind::Var => decl.decls.iter()
                    .flat_map(|decl| binding_names(&decl.binding)).collect::<Vec<_>>(),
                StmtKind::Class(class) => class.name.iter().collect(),
                _ => Vec::new(),
            };
            if let Some(name) = lexical.into_iter().find(|name| parameter_names.contains(&name.name)) {
                return Err(self.err_at(name.span.start, "parameter conflicts with lexical binding"));
            }
        }
        Ok(())
    }

    pub(super) fn parse_params(&mut self) -> Result<Vec<Param>, SyntaxError> {
        self.in_formal_params += 1;
        let result = self.parse_params_inner();
        self.in_formal_params -= 1;
        result
    }

    pub(super) fn parse_params_inner(&mut self) -> Result<Vec<Param>, SyntaxError> {
        self.expect_punct(Punct::LParen)?;
        let mut params = Vec::new();
        if !self.at_punct(Punct::RParen) {
            loop {
                let pstart = self.cur_loc();
                let rest = self.eat_punct(Punct::Ellipsis);
                let binding = self.parse_binding()?;
                let default = if !rest && self.eat_punct(Punct::Eq) {
                    let saved = self.no_in;
                    self.no_in = true;
                    let d = self.parse_assignment_expr();
                    self.no_in = saved;
                    Some(d?)
                } else {
                    None
                };
                params.push(Param {
                    binding,
                    default,
                    rest,
                    span: self.span_from(pstart),
                });
                if !self.eat_punct(Punct::Comma) {
                    break;
                }
                if rest {
                    return Err(self.err("rest parameter must be last and cannot have a trailing comma"));
                }
                if self.at_punct(Punct::RParen) {
                    break;
                }
            }
        }
        self.expect_punct(Punct::RParen)?;
        Ok(params)
    }

    /// Method definitions use UniqueFormalParameters. Their body directive
    /// also rules out non-simple parameters, even when the containing script
    /// was already strict.
    pub(super) fn validate_method_params(
        &self,
        params: &[Param],
        body: &Block,
        strict: bool,
        own_strict_directive: bool,
    ) -> Result<(), SyntaxError> {
        let mut names = HashSet::new();
        for param in params {
            if own_strict_directive
                && (param.rest
                    || param.default.is_some()
                    || !matches!(param.binding, Binding::Ident(_)))
            {
                return Err(self.err_at(
                    param.span.start,
                    "a use strict directive cannot accompany non-simple parameters",
                ));
            }
            for ident in binding_names(&param.binding) {
                if !names.insert(&ident.name) {
                    return Err(self.err_at(ident.span.start, "duplicate method parameter"));
                }
                if strict && (ident.name == "eval" || ident.name == "arguments") {
                    return Err(self.err_at(
                        ident.span.start,
                        "eval and arguments cannot be strict parameters",
                    ));
                }
            }
        }
        let parameter_names: HashSet<_> = params.iter().flat_map(|param| binding_names(&param.binding))
            .map(|name| name.name.clone()).collect();
        for statement in &body.body {
            let lexical = match &statement.kind {
                StmtKind::Var(decl) if decl.kind != VarKind::Var => decl.decls.iter()
                    .flat_map(|decl| binding_names(&decl.binding)).collect::<Vec<_>>(),
                StmtKind::Class(class) => class.name.iter().collect(),
                _ => Vec::new(),
            };
            if let Some(name) = lexical.into_iter().find(|name| parameter_names.contains(&name.name)) {
                return Err(self.err_at(name.span.start, "parameter conflicts with lexical binding"));
            }
        }
        Ok(())
    }

    /// Runs `f` with the function-body context flags applied; used by every
    /// callable body so nested `return`/`yield`/`await` validity works.
    pub(super) fn with_fn_context<T>(
        &mut self,
        is_async: bool,
        is_generator: bool,
        f: impl FnOnce(&mut Self) -> Result<T, SyntaxError>,
    ) -> Result<T, SyntaxError> {
        self.in_function += 1;
        self.in_async += usize::from(is_async);
        self.in_generator += usize::from(is_generator);
        let saved_static_block = self.in_static_block;
        self.in_static_block = 0;
        let r = f(self);
        self.in_static_block = saved_static_block;
        self.in_generator -= usize::from(is_generator);
        self.in_async -= usize::from(is_async);
        self.in_function -= 1;
        r
    }

    // ----- classes -----------------------------------------------------------

    /// Parses everything after the `class` keyword (consumed here).
    pub(super) fn parse_class_rest(&mut self) -> Result<ClassItem, SyntaxError> {
        let saved_strict = self.strict_script;
        self.strict_script = true;
        let result = self.parse_class_rest_inner();
        self.strict_script = saved_strict;
        result
    }

    fn parse_class_rest_inner(&mut self) -> Result<ClassItem, SyntaxError> {
        self.bump(); // class
        let name = if matches!(self.cur().kind, TokenKind::Ident(_)) {
            let id = self.expect_ident()?;
            if matches!(id.name.as_str(), "eval" | "arguments" | "implements" | "interface" | "let" | "package" | "private" | "protected" | "public" | "static" | "yield") {
                return Err(self.err_at(id.span.start, "invalid class name in strict mode"));
            }
            Some(id)
        } else {
            None
        };
        let super_class = if self.eat_kw(crate::tokens::Keyword::Extends) {
            let heritage = self.parse_assignment_expr()?;
            if matches!(heritage.kind, ExprKind::Arrow(_)) {
                return Err(self.err_at(heritage.span.start, "class heritage cannot be an unparenthesized arrow"));
            }
            Some(heritage)
        } else {
            None
        };
        self.expect_punct(Punct::Lbrace)?;
        self.private_scopes.push(PrivateScope::default());
        self.class_heritage.push(super_class.is_some());
        let mut members = Vec::new();
        while !self.at_punct(Punct::Rbrace) {
            if self.at_eof() {
                return Err(self.err("unterminated class body: expected `}`"));
            }
            if self.word("static") && self.nth_punct(1, Punct::Lbrace) {
                self.bump();
                let saved = (self.in_function, self.in_generator, self.in_async,
                    self.in_new_target, self.loop_depth, self.switch_depth,
                    std::mem::take(&mut self.labels));
                self.in_function = 0;
                self.in_generator = 0;
                self.in_async = 0;
                self.in_new_target = 0;
                self.loop_depth = 0;
                self.switch_depth = 0;
                self.in_static_block += 1;
                let saved_super_property = self.super_property_allowed;
                self.super_property_allowed = true;
                let block = self.parse_block();
                self.super_property_allowed = saved_super_property;
                self.in_static_block -= 1;
                (self.in_function, self.in_generator, self.in_async,
                    self.in_new_target, self.loop_depth, self.switch_depth, self.labels) = saved;
                let block = block?;
                self.validate_scope_names(&block.body, true)?;
                self.validate_strict_statements(&block.body)?;
                members.push(ClassMember::StaticBlock(block));
                continue;
            }
            if self.eat_punct(Punct::Semi) {
                continue;
            }
            members.push(self.parse_class_member()?);
        }
        self.bump(); // }
        self.class_heritage.pop();
        let mut constructor_seen = false;
        for member in &members {
            if let ClassMember::Method(method) = member {
                if method.kind == MethodKind::Constructor {
                    if constructor_seen { return Err(self.err_at(method.span.start, "duplicate class constructor")); }
                    constructor_seen = true;
                }
            }
        }
        let scope = self.private_scopes.pop().expect("class scope");
        for use_name in scope.uses {
            if !scope.declared.contains_key(&use_name.name) {
                if let Some(outer) = self.private_scopes.last_mut() {
                    outer.uses.push(use_name);
                } else {
                    return Err(self.err_at(
                        use_name.span.start,
                        format!("private name {} is not declared", use_name.name),
                    ));
                }
            }
        }
        Ok(ClassItem {
            name,
            super_class,
            members,
        })
    }

    /// Can token `n` begin a class-member key? Guards contextual modifiers
    /// (`static`, `get`, `set`, `async`) from being eaten when they are
    /// themselves keys (`static() {}`, `get: 1`, `set = 3`, `static;`).
    pub(super) fn nth_starts_member_key(&self, n: usize) -> bool {
        match self.nth(n).kind {
            TokenKind::Ident(_)
            | TokenKind::Keyword(_)
            | TokenKind::Str(_)
            | TokenKind::Numeric(_) => true,
            TokenKind::PrivateIdent(_) => true,
            TokenKind::Punct(Punct::LBracket) | TokenKind::Punct(Punct::Star) => true,
            _ => false,
        }
    }

    pub(super) fn parse_class_member(&mut self) -> Result<ClassMember, SyntaxError> {
        let start = self.cur_loc();
        let is_static = self.word("static") && self.nth_starts_member_key(1);
        if is_static {
            self.bump();
        }
        let is_accessor = self.word("accessor") && !self.nth(1).newline_before
            && self.nth_starts_member_key(1);
        if is_accessor { self.bump(); }
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
            if self.word("get") && !self.nth(1).newline_before && self.nth_starts_member_key(1) {
                self.bump();
                kind = MethodKind::Get;
            } else if self.word("set") && !self.nth(1).newline_before && self.nth_starts_member_key(1) {
                self.bump();
                kind = MethodKind::Set;
            }
        }
        let (key, key_utf16) = if let TokenKind::PrivateIdent(name) = self.cur().kind.clone() {
            let t = self.bump();
            (PropKey::Private(Ident::new(name, t.span)), None)
        } else {
            self.parse_prop_key()?
        };
        if self.at_punct(Punct::LParen) {
            if is_accessor { return Err(self.err_at(start, "accessor fields cannot be methods")); }
            if is_static && prop_key_name(&key) == Some("prototype") {
                return Err(self.err_at(start, "static class methods cannot be named prototype"));
            }
            if !is_static && kind != MethodKind::Method
                && prop_key_name(&key) == Some("constructor") {
                return Err(self.err_at(start, "constructor cannot be an accessor"));
            }
            if !is_static
                && (is_async || is_generator)
                && prop_key_name(&key) == Some("constructor")
            {
                return Err(self.err_at(start, "class constructors cannot be async or generators"));
            }
            let saved_super_property = self.super_property_allowed;
            self.super_property_allowed = true;
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
                    self.super_property_allowed = saved_super_property;
                    return Err(error);
                }
            };
            let actual_kind = if kind == MethodKind::Method
                && !is_static
                && prop_key_name(&key) == Some("constructor")
            {
                MethodKind::Constructor
            } else {
                kind
            };
            self.validate_accessor_arity(actual_kind, &params, start)?;
            self.declare_private(&key, actual_kind, is_static)?;
            let saved_field = self.in_class_field_initializer;
            let saved_super = self.super_call_allowed;
            self.in_class_field_initializer = 0;
            self.super_call_allowed = actual_kind == MethodKind::Constructor
                && self.class_heritage.last().copied().unwrap_or(false);
            let body = self.with_fn_context(is_async, is_generator, |p| p.parse_block());
            self.in_new_target -= 1;
            self.in_static_block = saved_static_block;
            self.super_call_allowed = saved_super;
            self.super_property_allowed = saved_super_property;
            self.in_class_field_initializer = saved_field;
            let body = body?;
            self.validate_method_params(&params, &body, true, has_use_strict(&body))?;
            self.validate_strict_statements(&body.body)?;
            return Ok(ClassMember::Method(ClassMethod {
                key,
                key_utf16,
                kind: actual_kind,
                is_static,
                is_generator,
                is_async,
                params,
                body,
                span: self.span_from(start),
            }));
        }
        // Field definition (storage only).
        if is_generator || is_async || kind != MethodKind::Method {
            return Err(self.err(
                "only methods may use `*`, `async`, `get`, or `set` without a parameter list",
            ));
        }
        if prop_key_name(&key) == Some("constructor") {
            return Err(self.err_at(start, "class fields cannot be named constructor"));
        }
        if is_static && prop_key_name(&key) == Some("prototype") {
            return Err(self.err_at(start, "static class fields cannot be named prototype"));
        }
        self.declare_private(&key, MethodKind::Method, is_static)?;
        let init = if self.eat_punct(Punct::Eq) {
            self.in_class_field_initializer += 1;
            let saved_super_property = self.super_property_allowed;
            self.super_property_allowed = true;
            let init = self.parse_assignment_expr();
            self.super_property_allowed = saved_super_property;
            self.in_class_field_initializer -= 1;
            Some(init?)
        } else {
            None
        };
        // Field separator: `;`, `}`, or ASI (newline). Anything else is an
        // error (`{ a = 1 b = 2 }` is invalid).
        if !self.at_punct(Punct::Rbrace) && !self.cur().newline_before {
            self.expect_punct(Punct::Semi)?;
        } else {
            self.eat_punct(Punct::Semi);
        }
        Ok(ClassMember::Field(ClassField {
            key,
            key_utf16,
            is_static,
            is_accessor,
            init,
            span: self.span_from(start),
        }))
    }

    pub(super) fn declare_private(
        &mut self,
        key: &PropKey,
        kind: MethodKind,
        is_static: bool,
    ) -> Result<(), SyntaxError> {
        let PropKey::Private(id) = key else {
            return Ok(());
        };
        if id.name == "#constructor" {
            return Err(self.err_at(id.span.start, "#constructor is not a valid private name"));
        }
        let bit = match kind {
            MethodKind::Get => 1,
            MethodKind::Set => 2,
            _ => 4,
        };
        let scope = self.private_scopes.last_mut().expect("class scope");
        let previous = scope
            .declared
            .entry(id.name.clone())
            .or_insert((0, is_static));
        if previous.1 != is_static
            || previous.0 & bit != 0
            || (previous.0 | bit) & 4 != 0 && previous.0 != 0
        {
            return Err(self.err_at(id.span.start, format!("duplicate private name {}", id.name)));
        }
        previous.0 |= bit;
        Ok(())
    }

    pub(super) fn validate_accessor_arity(
        &self,
        kind: MethodKind,
        params: &[Param],
        at: SourceLocation,
    ) -> Result<(), SyntaxError> {
        match kind {
            MethodKind::Get if !params.is_empty() => {
                Err(self.err_at(at, "getter must have no parameters"))
            }
            MethodKind::Set if params.len() != 1 || params[0].rest => {
                Err(self.err_at(at, "setter must have exactly one non-rest parameter"))
            }
            _ => Ok(()),
        }
    }

    pub(super) fn use_private(&mut self, id: Ident) -> Result<(), SyntaxError> {
        let Some(scope) = self.private_scopes.last_mut() else {
            return Err(self.err_at(id.span.start, "private name outside class"));
        };
        scope.uses.push(id);
        Ok(())
    }

    /// Property key: identifier (reserved words included), string, numeric
    /// literal, or `[expr]`.
    pub(super) fn parse_prop_key(&mut self) -> Result<(PropKey, Option<Vec<u16>>), SyntaxError> {
        match self.cur().kind.clone() {
            TokenKind::Ident(name) => {
                let token = self.bump();
                Ok((PropKey::Ident(Ident::new(name, token.span)), None))
            }
            TokenKind::Keyword(k) => {
                let t = self.bump();
                Ok((PropKey::Ident(Ident::new(k.spelling(), t.span)), None))
            }
            TokenKind::Str(s) => {
                let token = self.bump();
                Ok((PropKey::Str(s), token.cooked_utf16))
            }
            TokenKind::Numeric(n) => {
                self.bump();
                Ok((PropKey::Num(n), None))
            }
            TokenKind::Punct(Punct::LBracket) => {
                self.bump();
                let saved_no_in = self.no_in;
                self.no_in = false;
                let e = self.parse_assignment_expr();
                self.no_in = saved_no_in;
                let e = e?;
                self.expect_punct(Punct::RBracket)?;
                Ok((PropKey::Computed(Box::new(e)), None))
            }
            other => Err(self.err(format!(
                "expected property name, found {}",
                other.describe()
            ))),
        }
    }
}
